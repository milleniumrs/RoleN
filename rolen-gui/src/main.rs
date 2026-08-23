//! The `rolen-gui` binary: a thin winit + glutin + glow shell around the
//! `rolen_gui` library.
//!
//! Everything the window draws is decided by [`rolen_gui::app::RoleNApp`];
//! this file only owns the OS window, the OpenGL context, and the event loop.
//!
//! Threading contract: the poller and the job workers run on their own
//! threads and hand results to the UI through channels; when a result lands
//! they invoke the [`rolen_gui::Wake`] callback, which posts a user event
//! here so the loop redraws. The loop itself sleeps (`ControlFlow::Wait`)
//! between events - nothing repaints continuously.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use dear_imgui_glow::GlowRenderer;
use dear_imgui_rs::{Context, Theme};
use dear_imgui_winit::WinitPlatform;
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentGlContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface};
use raw_window_handle::HasWindowHandle;
use rolen_gui::app::RoleNApp;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct ImguiState {
    context: Context,
    platform: WinitPlatform,
    renderer: GlowRenderer,
    last_frame: Instant,
}

struct Shell {
    imgui: ImguiState,
    app: RoleNApp,
    gl_context: glutin::context::PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Arc<Window>,
}

impl Shell {
    fn new(
        event_loop: &ActiveEventLoop,
        wake: rolen_gui::Wake,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let window_attributes = Window::default_attributes()
            .with_title("RoleN")
            .with_inner_size(LogicalSize::new(1280.0, 820.0))
            .with_min_inner_size(LogicalSize::new(960.0, 600.0));

        let (window, cfg) = glutin_winit::DisplayBuilder::new()
            .with_window_attributes(Some(window_attributes))
            .build(event_loop, ConfigTemplateBuilder::new(), |mut configs| {
                configs.next().expect("no GL framebuffer config available")
            })?;
        let window = Arc::new(window.expect("DisplayBuilder did not create a window"));

        let context_attribs =
            ContextAttributesBuilder::new().build(Some(window.window_handle()?.as_raw()));
        let context = unsafe { cfg.display().create_context(&cfg, &context_attribs)? };

        let size = window.inner_size();
        let surface_attribs = SurfaceAttributesBuilder::<WindowSurface>::new()
            .with_srgb(Some(true))
            .build(
                window.window_handle()?.as_raw(),
                NonZeroU32::new(size.width.max(1)).unwrap(),
                NonZeroU32::new(size.height.max(1)).unwrap(),
            );
        let surface = unsafe {
            cfg.display()
                .create_window_surface(&cfg, &surface_attribs)?
        };
        let gl_context = context.make_current(&surface)?;

        let mut context = Context::create();
        // Window layouts are session state, not user data; do not write
        // imgui.ini next to the binary.
        context.set_ini_filename(None::<String>)?;
        // The built-in font is 13 px; scale it up for desktop readability.
        context.style_mut().set_font_scale_main(1.25);

        let mut platform = WinitPlatform::new(&mut context)?;
        platform.attach_window(
            Arc::clone(&window),
            dear_imgui_winit::HiDpiMode::Default,
            &mut context,
        )?;

        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|s| {
                gl_context.display().get_proc_address(s).cast()
            })
        };
        let mut renderer = GlowRenderer::new(gl, &mut context)?;
        renderer.set_framebuffer_srgb_enabled(true)?;

        let mut app = RoleNApp::new(wake);
        // Apply the startup theme (dark) through the same path as menu changes.
        if let Some(preset) = app.take_theme_change() {
            Theme {
                preset,
                ..Default::default()
            }
            .apply_to_context(&mut context);
        }

        Ok(Self {
            imgui: ImguiState {
                context,
                platform,
                renderer,
                last_frame: Instant::now(),
            },
            app,
            gl_context,
            surface,
            window,
        })
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.surface.resize(
                &self.gl_context,
                NonZeroU32::new(new_size.width).unwrap(),
                NonZeroU32::new(new_size.height).unwrap(),
            );
        }
    }

    fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let now = Instant::now();
        let delta = (now - self.imgui.last_frame).as_secs_f32().max(1e-6);
        self.imgui.context.io_mut().set_delta_time(delta);
        self.imgui.last_frame = now;

        if let Some(preset) = self.app.take_theme_change() {
            Theme {
                preset,
                ..Default::default()
            }
            .apply_to_context(&mut self.imgui.context);
        }

        self.imgui
            .platform
            .prepare_frame(&mut self.imgui.context, &self.window)?;

        {
            let ui = self.imgui.context.frame();
            self.app.draw(ui);

            if let Some(gl) = self.imgui.renderer.gl_context() {
                unsafe {
                    gl.enable(glow::FRAMEBUFFER_SRGB);
                    gl.clear_color(0.11, 0.11, 0.12, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
            }
            self.imgui.platform.prepare_render(ui, &self.window)?;
        }

        let pending_frame = self
            .imgui
            .context
            .render(self.imgui.renderer.renderer_consumer()?);
        self.imgui.renderer.render(pending_frame)?;
        self.surface.swap_buffers(&self.gl_context)?;
        Ok(())
    }

    fn shutdown(&mut self) {
        self.imgui.context.end_frame();
        if let Err(e) = self.imgui.renderer.shutdown(&mut self.imgui.context) {
            eprintln!("glow renderer shutdown failed: {e}");
        }
        if let Err(e) = self.imgui.platform.shutdown(&mut self.imgui.context) {
            eprintln!("winit platform shutdown failed: {e}");
        }
        if let Err(e) = self.gl_context.make_not_current_in_place() {
            eprintln!("GL context unbind failed: {e}");
        }
    }
}

#[derive(Default)]
struct App {
    shell: Option<Shell>,
    wake: Option<rolen_gui::Wake>,
}

impl ApplicationHandler<()> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.shell.is_some() {
            return;
        }
        let wake = self.wake.take().unwrap_or_else(rolen_gui::wake::no_op);
        match Shell::new(event_loop, wake) {
            Ok(shell) => {
                shell.window.request_redraw();
                self.shell = Some(shell);
            }
            Err(e) => {
                eprintln!("could not start the GUI: {e}");
                event_loop.exit();
            }
        }
    }

    /// A background thread (poller or job) has news; redraw once.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(shell) = &self.shell {
            shell.window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(shell) = self.shell.as_mut() else {
            return;
        };

        if let Err(e) = shell.imgui.platform.handle_window_event(
            &mut shell.imgui.context,
            &shell.window,
            &event,
        ) {
            eprintln!("winit platform event error: {e}");
            event_loop.exit();
            return;
        }

        match event {
            WindowEvent::Resized(size) => shell.resize(size),
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(e) = shell.render() {
                    eprintln!("render error: {e}");
                }
                if shell.app.take_exit_request() {
                    event_loop.exit();
                    return;
                }
            }
            _ => {}
        }

        // Any input event can change the UI; repaint once after handling it.
        // (RedrawRequested is excluded to avoid an immediate re-render loop.)
        if !matches!(event, WindowEvent::RedrawRequested) {
            shell.window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(shell) = self.shell.as_mut() {
            shell.shutdown();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop: EventLoop<()> = EventLoop::with_user_event().build()?;
    // Sleep between events; the poller and job workers post a user event
    // through this proxy when there is something new to draw.
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let wake: rolen_gui::Wake = Arc::new(move || {
        let _ = proxy.send_event(());
    });

    let mut app = App {
        shell: None,
        wake: Some(wake),
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
