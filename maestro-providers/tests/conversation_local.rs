//! End-to-end check that a conversation actually carries context.
//!
//! Ignored by default: it needs a running Ollama on localhost:11434 and an
//! `ollama-local` provider in the registry, so it cannot run in CI. Run it by
//! hand after touching the chat path:
//!
//! ```text
//! cargo test -p maestro-providers --test conversation_local -- --ignored --nocapture
//! ```
//!
//! This is the regression guard for the defect the TUI shipped with: every
//! send was a fresh single-message request, so the model never saw what had
//! already been said and the second question below could not be answered.

use maestro_providers::chat::{ChatMessage, ChatRequest};
use maestro_providers::conversation::{self, Totals};

const PROVIDER: &str = "ollama-local";
const MODEL: &str = "phi3:mini";

#[test]
#[ignore = "requires a local Ollama and the ollama-local provider"]
fn the_model_sees_the_previous_turn() {
    let session = conversation::new_session_id();
    let mut history = vec![ChatMessage::user(
        "My name is Vlad. Remember it. Reply with exactly: ok",
    )];

    let first = conversation::send(
        PROVIDER,
        MODEL,
        history.clone(),
        &session,
        Totals::default(),
        64,
    )
    .expect("first turn should reach the local model");
    println!("turn 1: {}", first.text.trim());
    history.push(ChatMessage::assistant(first.text.trim()));

    // Only answerable if the first turn was actually sent along.
    history.push(ChatMessage::user(
        "What is my name? Reply with just the name.",
    ));
    let second = conversation::send(
        PROVIDER,
        MODEL,
        history,
        &session,
        Totals {
            tokens_in: first.tokens_in,
            tokens_out: first.tokens_out,
            cost: first.cost,
        },
        64,
    )
    .expect("second turn should reach the local model");
    println!("turn 2: {}", second.text.trim());

    assert!(
        second.text.to_lowercase().contains("vlad"),
        "the model did not receive the earlier turn; it replied {:?}",
        second.text
    );
}

/// The same question asked the old way - one message, no history - must fail
/// to answer. Without this the test above could pass for the wrong reason.
#[test]
#[ignore = "requires a local Ollama and the ollama-local provider"]
fn a_single_message_request_cannot_answer_it() {
    let reg = maestro_providers::ProviderRegistry::load().expect("registry");
    let provider = reg.get(PROVIDER).expect("ollama-local registered").clone();

    // What ChatRequest::single builds, and what the TUI used to send.
    let request = ChatRequest::single(MODEL, "What is my name? Reply with just the name.");
    let response = maestro_providers::client::chat(&provider, &request).expect("chat");
    println!("no-history reply: {}", response.text.trim());

    assert!(
        !response.text.to_lowercase().contains("vlad"),
        "the model somehow knew a name it was never told: {:?}",
        response.text
    );
}
