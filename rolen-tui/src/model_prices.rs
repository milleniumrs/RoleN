//! Model prices window (FR-1.5): USD per million tokens for every model of
//! every registered provider.
//!
//! Rows carry their own provider/model ids rather than relying on the row
//! index, because the list is sortable and searchable — an index would point
//! at the wrong model as soon as a column header is clicked.

use appcui::prelude::*;
use rolen_core::pricing::{fmt_rate, Price, Pricing};
use rolen_providers as providers;

#[derive(ListItem)]
struct PriceRow {
    #[Column(name: "&Provider", width: 18)]
    provider: String,
    #[Column(name: "&Model", width: 32)]
    model: String,
    #[Column(name: "Input $/Mtok", width: 14, align: right)]
    price_in: String,
    #[Column(name: "Output $/Mtok", width: 14, align: right)]
    price_out: String,
    // Not columns: what this row points at.
    provider_id: String,
    model_id: String,
    metered: bool,
}

#[ModalWindow(events = ButtonEvents+ListViewEvents<PriceRow>)]
pub struct ModelPrices {
    l_info: Handle<Label>,
    lv: Handle<ListView<PriceRow>>,
    b_edit: Handle<Button>,
    b_clear: Handle<Button>,
    b_close: Handle<Button>,
}

impl ModelPrices {
    pub fn new() -> Self {
        let mut w = Self {
            base: ModalWindow::new(
                "Model Prices (USD per million tokens)",
                layout!("a:c,w:88,h:26"),
                window::Flags::Sizeable,
            ),
            l_info: Handle::None,
            lv: Handle::None,
            b_edit: Handle::None,
            b_clear: Handle::None,
            b_close: Handle::None,
        };
        w.l_info = w.add(label!("x:1,y:0,w:84,h:2,text:''"));
        w.lv = w.add(listview!(
            "class: PriceRow,l:1,t:2,r:1,b:3,flags: [ScrollBars, SearchBar]"
        ));
        w.add(label!(
            "'Enter on a row edits its price. Local Ollama is free and cannot be priced.',l:1,b:2,r:1"
        ));
        w.b_edit = w.add(button!("'&Edit price',l:14,b:0,w:16"));
        w.b_clear = w.add(button!("'&Clear price',l:34,b:0,w:16"));
        w.b_close = w.add(button!("'C&lose',l:54,b:0,w:16"));
        w.refresh();
        w
    }

    fn refresh(&mut self) {
        let reg = providers::ProviderRegistry::load().unwrap_or_default();
        let pricing = Pricing::load().unwrap_or_default();

        let mut rows = Vec::new();
        let (mut free, mut known, mut unknown) = (0usize, 0usize, 0usize);
        for p in reg.list() {
            for m in &p.models {
                let price = pricing.resolve(p.ptype, &p.id, &m.id);
                match price {
                    Price::Free => free += 1,
                    Price::Known { .. } => known += 1,
                    Price::Unknown => unknown += 1,
                }
                rows.push(PriceRow {
                    provider: p.id.clone(),
                    model: m.id.clone(),
                    price_in: price.input_label(),
                    price_out: price.output_label(),
                    provider_id: p.id.clone(),
                    model_id: m.id.clone(),
                    metered: p.ptype.is_metered(),
                });
            }
        }

        let info = if reg.is_empty() {
            "No providers registered. Use Providers > Add Provider or Detect first.".to_string()
        } else if rows.is_empty() {
            "No models discovered yet. Use Providers > Detect, or open a provider and discover its models.".to_string()
        } else {
            let path = rolen_core::config::pricing_file()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "pricing.toml".into());
            format!(
                "{} models — {known} priced, {free} free, {unknown} unknown\nPrices are stored in {path}",
                rows.len()
            )
        };

        let h = self.l_info;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(&info);
        }
        let h = self.lv;
        if let Some(lv) = self.control_mut(h) {
            lv.clear();
            for r in rows {
                lv.add(r);
            }
        }
    }

    /// The row under the cursor, as owned data: the list view borrow has to be
    /// released before anything can open a dialog or write the price file.
    fn selection(&self) -> Option<(String, String, bool)> {
        self.control(self.lv)
            .and_then(|lv| lv.current_item())
            .map(|r| (r.provider_id.clone(), r.model_id.clone(), r.metered))
    }

    fn edit_selected(&mut self) {
        let Some((provider_id, model_id, metered)) = self.selection() else {
            return;
        };
        if !metered {
            dialogs::message(
                "Model price",
                &format!(
                    "'{model_id}' runs on hardware you already pay for, so there is no per-token price to set.\n\nIt is reported as free."
                ),
            );
            return;
        }

        let pricing = Pricing::load().unwrap_or_default();
        let current = pricing.get(&provider_id, &model_id);
        let def_in = current.map(|e| fmt_rate(e.in_per_mtok));
        let def_out = current.map(|e| fmt_rate(e.out_per_mtok));

        let Some(input) = dialogs::input::<String>(
            "Model price",
            &format!("{provider_id}/{model_id}\nUSD per 1M input tokens:"),
            def_in,
            None,
        ) else {
            return;
        };
        let Some(output) = dialogs::input::<String>(
            "Model price",
            &format!("{provider_id}/{model_id}\nUSD per 1M output tokens:"),
            def_out,
            None,
        ) else {
            return;
        };

        let rate_in = match parse_rate(&input) {
            Ok(v) => v,
            Err(e) => return dialogs::error("Model price", &format!("input rate: {e}")),
        };
        let rate_out = match parse_rate(&output) {
            Ok(v) => v,
            Err(e) => return dialogs::error("Model price", &format!("output rate: {e}")),
        };

        let mut pricing = pricing;
        pricing.set(&provider_id, &model_id, rate_in, rate_out);
        match pricing.save() {
            Ok(()) => self.refresh(),
            Err(e) => dialogs::error("Model price", &format!("could not save: {e}")),
        }
    }

    fn clear_selected(&mut self) {
        let Some((provider_id, model_id, metered)) = self.selection() else {
            return;
        };
        if !metered {
            return;
        }
        let mut pricing = Pricing::load().unwrap_or_default();
        if !pricing.clear(&provider_id, &model_id) {
            return;
        }
        match pricing.save() {
            Ok(()) => self.refresh(),
            Err(e) => dialogs::error("Model price", &format!("could not save: {e}")),
        }
    }
}

/// Accept "3", "3.00" or "$3.00"; reject anything that is not a rate.
fn parse_rate(text: &str) -> Result<f64, String> {
    let t = text.trim().trim_start_matches('$').trim();
    if t.is_empty() {
        return Err("enter a number, for example 3.00".into());
    }
    let v: f64 = t.parse().map_err(|_| format!("'{t}' is not a number"))?;
    if !v.is_finite() {
        return Err("not a finite number".into());
    }
    if v < 0.0 {
        return Err("a price cannot be negative".into());
    }
    Ok(v)
}

impl ListViewEvents<PriceRow> for ModelPrices {
    fn on_item_action(
        &mut self,
        _handle: Handle<ListView<PriceRow>>,
        _index: usize,
    ) -> EventProcessStatus {
        self.edit_selected();
        EventProcessStatus::Processed
    }
}

impl ButtonEvents for ModelPrices {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_edit {
            self.edit_selected();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_clear {
            self.clear_selected();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_close {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::parse_rate;

    #[test]
    fn plain_and_dollar_prefixed_rates_both_parse() {
        assert_eq!(parse_rate("3").unwrap(), 3.0);
        assert_eq!(parse_rate("3.00").unwrap(), 3.0);
        assert_eq!(parse_rate("$3.00").unwrap(), 3.0);
        assert_eq!(parse_rate("  $0.075 ").unwrap(), 0.075);
    }

    #[test]
    fn zero_is_a_valid_rate() {
        assert_eq!(parse_rate("0").unwrap(), 0.0);
    }

    #[test]
    fn junk_and_negatives_are_rejected() {
        assert!(parse_rate("").is_err());
        assert!(parse_rate("$").is_err());
        assert!(parse_rate("free").is_err());
        assert!(parse_rate("-1").is_err());
        assert!(parse_rate("NaN").is_err());
        assert!(parse_rate("inf").is_err());
    }
}
