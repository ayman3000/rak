//! Falling back to another provider: the chain, and the hook the agent asks
//! when the provider in use fails. The chain is built from the key file,
//! with no settings of its own:
//!
//! 1. the provider in use comes first (it is the one that failed);
//! 2. then each other provider whose key is set, in the order the key lines
//!    appear in the .env files, then the servers on this machine;
//! 3. each with the model last picked for it; a provider never used in rak
//!    (no model picked) is skipped: a key in the file is not consent to spend.

use super::args::Options;
use super::output::Ui;
use super::session::{backend, make_provider, window_for, Backend};
use super::style::{paint, Style};
use rak::delegate::Offer;
use rak::fallback::{Failure, Hook, Next, Switch};
use rak::interrupt::Stop;
use rak::providers::Provider;
use rak::text::one_line;
use std::cell::{Cell, RefCell};
use std::io::{BufRead, Write};
use std::rc::Rc;

/// One provider to try, and the model to try it with.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub provider: String,
    pub model: String,
}

/// Pure — unit-tested. The providers to fall back to after `current`, in order.
pub fn chain(
    all: &[Provider],
    current: Option<&str>,
    key_order: &[String],
    is_set: &dyn Fn(&str) -> bool,
    last_pick: &dyn Fn(&str) -> Option<String>,
) -> Vec<Link> {
    let mut keyed: Vec<(usize, usize, &Provider)> = Vec::new();
    let mut local: Vec<&Provider> = Vec::new();
    for (index, p) in all.iter().enumerate() {
        if current.is_some_and(|c| c.eq_ignore_ascii_case(&p.name)) {
            continue;
        }
        match &p.key {
            Some(key) if is_set(key) => {
                let position = key_order.iter().position(|k| k == key).unwrap_or(usize::MAX);
                keyed.push((position, index, p));
            }
            Some(_) => {}
            None => local.push(p),
        }
    }
    keyed.sort_by_key(|(position, index, _)| (*position, *index));
    keyed
        .into_iter()
        .map(|(_, _, p)| p)
        .chain(local)
        .filter_map(|p| last_pick(&p.name).map(|model| Link { provider: p.name.clone(), model }))
        .collect()
}

/// For people: "anthropic · claude-sonnet-5-5".
pub fn label(backend: &Backend, model: &str) -> String {
    let at = backend.provider.clone().unwrap_or_else(|| backend.base_url.clone());
    format!("{at} · {model}")
}

/// Where the chain stands during a request.
#[derive(Default)]
pub struct State {
    pub links: Vec<Link>,
    pub next: usize,
    /// What the agent is using now, when that is a fallback.
    pub current: Option<(Options, Backend, String, usize)>,
    pub current_label: String,
    /// The provider chosen failed in a way that won't pass (a bad key, a
    /// missing model): the chat stays on the fallback.
    pub for_good: bool,
}

/// What the hook needs besides the chain.
#[derive(Clone)]
pub struct Asker {
    pub o: Options,
    pub stop: Stop,
    pub ui: Rc<RefCell<Ui>>,
    /// Auto mode (-y, /yes): switch without asking.
    pub auto: Rc<Cell<bool>>,
    /// Someone at a terminal to ask; without, rak switches and says so.
    pub interactive: bool,
    pub color: bool,
}

/// Pure — unit-tested. A yes to the fallback question.
pub fn is_yes(typed: &str) -> bool {
    matches!(typed.trim(), "y" | "Y" | "yes" | "Yes")
}

/// The hook for the agent: the next provider in the chain whose key is set,
/// asked about first in a chat.
pub fn hook(asker: Asker, state: Rc<RefCell<State>>) -> Hook {
    Box::new(move |why, failure| match offer(&asker, &mut state.borrow_mut(), why, failure) {
        Offer::Switch { model, context_window, label } => Next::Switch(Switch { model, context_window, label }),
        Offer::NoneLeft => Next::NoneLeft,
        Offer::Declined => Next::Declined,
    })
}

/// The next model in the chain, after asking when there is someone to ask.
/// Its model can go to another thread (a delegate helper's).
pub fn offer(asker: &Asker, st: &mut State, why: &str, failure: Failure) -> Offer {
    while st.next < st.links.len() {
        let link = st.links[st.next].clone();
        st.next += 1;
        let mut o = asker.o.clone();
        o.provider = Some(link.provider.clone());
        o.provider_flag = true;
        (o.base_url, o.base_url_flag, o.api_key_env) = (None, false, None);
        let Ok(backend) = backend(&o, Some(&link.model)) else { continue };
        let to = label(&backend, &link.model);
        if !asker.auto.get() && asker.interactive {
            asker.ui.borrow_mut().end_line();
            let question = format!("{} failed: {}\n  Continue with {to}? [y/n] › ", st.current_label, one_line(why, 120));
            eprint!("{}", paint(asker.color, Style::Yellow, &question));
            asker.ui.borrow_mut().asked = true;
            let _ = std::io::stderr().flush();
            let mut typed = String::new();
            let _ = std::io::stdin().lock().read_line(&mut typed);
            if asker.stop.requested() || !is_yes(&typed) {
                asker.ui.borrow_mut().asked = false;
                return Offer::Declined;
            }
        }
        let model = make_provider(&o, &backend, &link.model, &asker.stop);
        let (window, _) = window_for(&o, model.as_ref());
        if failure == Failure::Config {
            st.for_good = true;
        }
        st.current = Some((o, backend, link.model.clone(), window));
        st.current_label = to.clone();
        return Offer::Switch { model, context_window: window, label: to };
    }
    Offer::NoneLeft
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, key: Option<&str>) -> Provider {
        Provider {
            name: name.into(),
            label: name.into(),
            base_url: format!("https://{name}"),
            key: key.map(str::to_string),
            anthropic: false,
        }
    }

    #[test]
    fn the_chain_follows_the_key_file_and_skips_what_was_never_used() {
        let all = [
            provider("ollama", None),
            provider("openai", Some("OPENAI_API_KEY")),
            provider("anthropic", Some("ANTHROPIC_API_KEY")),
            provider("groq", Some("GROQ_API_KEY")),
            provider("zai", Some("ZAI_API_KEY")),
            provider("lmstudio", None),
        ];
        // Keys set: anthropic, groq, openai (in this file order); zai's is not.
        let order: Vec<String> = ["GROQ_API_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"].iter().map(|s| s.to_string()).collect();
        let is_set = |k: &str| order.iter().any(|o| o == k);
        // Models picked before: everywhere but openai and lmstudio.
        let last = |p: &str| (!["openai", "lmstudio"].contains(&p)).then(|| format!("{p}-model"));
        let links = chain(&all, Some("anthropic"), &order, &is_set, &last);
        let names: Vec<&str> = links.iter().map(|l| l.provider.as_str()).collect();
        assert_eq!(names, ["groq", "ollama"], "file order; the current one left out; never used and no key skipped; local last");
        assert_eq!(links[0].model, "groq-model");
        assert!(chain(&all, Some("groq"), &[], &|_| false, &|_| None).is_empty());
    }

    #[test]
    fn only_a_yes_switches() {
        assert!(is_yes("y\n") && is_yes(" yes "));
        assert!(!is_yes("") && !is_yes("n") && !is_yes("maybe"));
    }
}
