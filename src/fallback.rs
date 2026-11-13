//! Falling back to another model when the one in use fails. The agent asks a
//! hook for the next model (the CLI builds the chain from the key file and
//! asks the person); this file says which failures are worth that, and what
//! the hook is given and gives back.

use crate::compact;
use crate::provider::{Model, ModelError, STOPPED};

/// Why a model call failed, as far as choosing what to do next goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Failure {
    /// Network, timeout, server error, overloaded: worth a retry, then the next model.
    Transient,
    /// Too many requests: worth a retry, then the next model.
    RateLimit,
    /// Out of credit or quota: retrying won't help; the next model may.
    Credit,
    /// A bad key, or a model the server doesn't have: this provider won't work
    /// until the config changes, so it is left for the rest of the chat.
    Config,
    /// Longer than the model takes, even after compacting.
    Overflow,
    /// The answer stopped part way, after some of it was shown.
    Midway,
    /// Stopped by the person, a refusal, or a request rak built wrongly:
    /// another model would not do better.
    Stop,
}

impl Failure {
    /// Whether another model may succeed where this one failed.
    pub fn falls_back(self) -> bool {
        !matches!(self, Failure::Stop)
    }

    /// Whether the same model is worth asking again first.
    pub fn retries(self) -> bool {
        matches!(self, Failure::Transient | Failure::RateLimit)
    }
}

/// Pure — unit-tested. What kind of failure an error from a provider is, from
/// the status code and words both providers' errors carry.
pub fn classify(error: &ModelError) -> Failure {
    let (retryable, text) = match error {
        ModelError::Retryable(text) => (true, text),
        ModelError::Fatal(text) => (false, text),
    };
    let lower = text.to_lowercase();
    let status = |code: &str| lower.contains(&format!("answered {code}"));
    let credit = ["insufficient", "quota", "credit", "balance", "billing", "payment required"].iter().any(|w| lower.contains(w));
    if text == STOPPED || lower.contains("declined") {
        Failure::Stop
    } else if lower.contains("stopped arriving part way") {
        Failure::Midway
    } else if compact::is_overflow(text) {
        Failure::Overflow
    } else if status("402") || credit {
        Failure::Credit
    } else if status("429") {
        Failure::RateLimit
    } else if retryable {
        Failure::Transient
    } else if status("401")
        || status("403")
        || status("404")
        || lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("no such model")
        || lower.contains("api key")
    {
        Failure::Config
    } else {
        Failure::Stop
    }
}

/// The model to carry on with.
pub struct Switch {
    pub model: Box<dyn Model>,
    pub context_window: usize,
    /// For people: "anthropic · claude-sonnet-5-5".
    pub label: String,
}

/// What the hook answers.
pub enum Next {
    Switch(Switch),
    /// Nothing left to try.
    NoneLeft,
    /// The person said no to switching: the request stops.
    Declined,
}

/// Asked for the next model: what failed (its error, as text) and why.
pub type Hook = Box<dyn FnMut(&str, Failure) -> Next>;

/// The last line of the error when the person said no to switching.
pub const DECLINED: &str = "Stopped: you chose not to switch to another model.";

/// Same-model retries before moving on, when there is somewhere to move on to.
pub const RETRIES_BEFORE_FALLBACK: usize = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn fatal(text: &str) -> Failure {
        classify(&ModelError::Fatal(text.into()))
    }

    fn retryable(text: &str) -> Failure {
        classify(&ModelError::Retryable(text.into()))
    }

    #[test]
    fn failures_are_sorted_by_what_can_help() {
        assert_eq!(retryable("could not reach https://ollama.com/v1: timed out"), Failure::Transient);
        assert_eq!(retryable("the model server answered 503: overloaded"), Failure::Transient);
        assert_eq!(retryable("the Anthropic API answered 529: overloaded"), Failure::Transient);
        assert_eq!(retryable("the model server answered 429: slow down"), Failure::RateLimit);
        assert_eq!(retryable("the model server answered 429: You exceeded your current quota"), Failure::Credit);
        assert_eq!(fatal("the model server answered 402: Payment Required"), Failure::Credit);
        assert_eq!(fatal("the Anthropic API answered 400: Your credit balance is too low"), Failure::Credit);
        assert_eq!(fatal("the Anthropic API answered 401: invalid x-api-key"), Failure::Config);
        assert_eq!(fatal("the model server answered 404: model 'kimi-k2.6:cloud' not found"), Failure::Config);
        assert_eq!(fatal("the model server reported an error: model \"x\" not found, try pulling it first"), Failure::Config);
        assert_eq!(fatal("the Anthropic API answered 400: prompt is too long"), Failure::Overflow);
        assert_eq!(fatal("the answer stopped arriving part way: connection reset"), Failure::Midway);
        assert_eq!(fatal("the model declined this request (cyber)"), Failure::Stop);
        assert_eq!(fatal(STOPPED), Failure::Stop);
        assert_eq!(fatal("the model server answered 400: tools.0.name: invalid"), Failure::Stop, "rak's own mistake");
        assert!(Failure::Transient.retries() && !Failure::Credit.retries() && !Failure::Stop.falls_back());
    }
}
