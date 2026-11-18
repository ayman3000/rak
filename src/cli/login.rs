//! `rak login chatgpt` and `rak logout chatgpt`: Sign in with ChatGPT, so the `chatgpt`
//! provider can use the person's ChatGPT plan. The browser does the signing
//! in; rak waits for its answer on 127.0.0.1 and keeps the tokens in
//! ~/.config/rak/chatgpt.json. The rules are in src/chatgpt/auth.rs.

use super::style::{paint, Style};
use rak::chatgpt::auth::{self, Account};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const USAGE: &str = "rak login chatgpt             sign in with ChatGPT, to use your ChatGPT plan (the chatgpt provider)
rak login chatgpt --new       sign in with another ChatGPT account or workspace
rak login chatgpt --status    say who is signed in; change nothing
rak logout chatgpt            sign out: the tokens are revoked and removed

The sign-in happens in your browser, which then answers rak on 127.0.0.1.
On a remote machine, open the address rak prints in a browser on a computer
that reaches it: forward the port first (ssh -L 1455:127.0.0.1:1455 HOST).
rak keeps the sign-in in ~/.config/rak/chatgpt.json, readable by you only.";

/// The port tried first; the callback's address may differ only in its port.
const PORT: u16 = 1455;
const PATH: &str = "/auth/callback";
/// How long the browser has to come back.
const WAIT: Duration = Duration::from_secs(300);

/// Pure — unit-tested. What follows `rak login` or `rak logout`: the service
/// named first (only `chatgpt` so far), then its own arguments. A missing or
/// unknown name is answered with what can be signed in to.
pub fn after_name<'a>(command: &str, args: &'a [String]) -> Result<Vec<&'a str>, String> {
    let known = "rak can sign in to: chatgpt (your ChatGPT plan). Providers with an API key take `rak keys set NAME` instead.";
    match args.first().map(String::as_str) {
        Some("chatgpt") => Ok(args[1..].iter().map(String::as_str).collect()),
        Some("-h" | "--help") => Ok(vec!["--help"]),
        Some(other) if !other.starts_with('-') => Err(format!("rak {command} does not know `{other}`. {known}")),
        _ => Err(format!("say what to sign {}: rak {command} chatgpt. {known}", if command == "login" { "in to" } else { "out of" })),
    }
}

fn named(command: &str, args: &[String]) -> Vec<String> {
    match after_name(command, args) {
        Ok(rest) => rest.into_iter().map(str::to_string).collect(),
        Err(why) => {
            eprintln!("rak: {why}");
            std::process::exit(2);
        }
    }
}

/// `rak login chatgpt …`.
pub fn login_command(args: &[String]) -> ! {
    let color = color();
    let args = named("login", args);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match args.as_slice() {
        ["-h"] | ["--help"] => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        ["--status"] => status(),
        [] => sign_in(color, false).map(|account| println!("{}", welcome(&account))),
        ["--new"] => sign_in(color, true).map(|account| println!("{}", welcome(&account))),
        _ => {
            eprintln!("rak: unknown arguments for rak login chatgpt\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    finish(outcome)
}

/// `rak logout chatgpt`.
pub fn logout_command(args: &[String]) -> ! {
    let args = named("logout", args);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => finish(sign_out().map(|said| println!("{said}"))),
        ["-h"] | ["--help"] => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        _ => {
            eprintln!("rak: rak logout chatgpt takes no arguments\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

fn finish(outcome: Result<(), String>) -> ! {
    std::process::exit(match outcome {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("rak: {why}");
            1
        }
    })
}

fn color() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn account_file() -> Result<std::path::PathBuf, String> {
    auth::file().ok_or_else(|| "no home folder to keep a ChatGPT sign-in in".to_string())
}

fn status() -> Result<(), String> {
    let account = auth::load(&account_file()?)?;
    if !account.signed_in() {
        println!("Not signed in to ChatGPT. Sign in with: rak login chatgpt");
    } else if !account.plan_allowed() {
        println!("Signed in as {}, but using the ChatGPT plan was not allowed. Run `rak login chatgpt` to allow it.", account.who());
    } else {
        println!("Signed in to ChatGPT as {}. Using ChatGPT plan; manage usage: {}", account.who(), auth::USAGE_URL);
    }
    Ok(())
}

/// Pure — unit-tested. What a sign-in ends with: who, what it means, and how
/// to use it.
pub fn welcome(account: &Account) -> String {
    format!(
        "Signed in as {}.\n\
         You're using your ChatGPT plan: what rak sends through the chatgpt provider uses your ChatGPT plan, not an API key.\n\
         Manage usage: {}\n\
         Use it with `rak --provider chatgpt`, with /provider chatgpt in a chat, or make it the default with `rak setup`.",
        account.who(),
        auth::USAGE_URL
    )
}

/// Signs in through the browser and saves the sign-in. `another`: a new
/// registration, for a different ChatGPT account or workspace.
pub fn sign_in(color: bool, another: bool) -> Result<Account, String> {
    if cfg!(windows) {
        return Err("signing in with ChatGPT is for Linux and macOS for now".to_string());
    }
    let file = account_file()?;
    let saved = auth::load(&file).unwrap_or_default();
    let mut account = if another { Account { host_id: saved.host_id.clone(), ..Default::default() } } else { saved };
    // This computer's id is chosen, and kept, before its first sign-in.
    if account.host_id.is_empty() {
        account.host_id = auth::new_host_id();
        if !another {
            auth::save(&file, &account)?;
        }
    }

    let listener = listen()?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}{PATH}");
    let (state, nonce, verifier) = (auth::random_token(), auth::random_token(), auth::random_token());
    // Signed in before without allowing plan use: the permissions are asked again.
    let consent = account.client_id.is_some() && !account.scopes.is_empty() && !account.plan_allowed();
    let url = auth::authorize_url(&auth::Attempt {
        client_id: account.client_id.as_deref(),
        redirect_uri: &redirect_uri,
        state: &state,
        nonce: &nonce,
        verifier: &verifier,
        host_id: &account.host_id,
        login_hint: account.email.as_deref(),
        consent,
    });
    println!("{}", paint(color, Style::Bold, "Continue with ChatGPT"));
    println!("Your browser opens to sign in. To use your plan in rak, allow \"Use your ChatGPT plan\" there.");
    println!("{}", paint(color, Style::Dim, &format!("If it does not open, go to:\n{url}")));
    open_browser(&url);
    println!("{}", paint(color, Style::Dim, "Waiting for the browser… (Ctrl-C to stop)"));

    let callback = wait_for_callback(&listener, &state)?;
    let client_id = issued_client(account.client_id.as_deref(), callback.client_id.as_deref())?;
    let tokens = auth::exchange(&client_id, &callback.code, &verifier, &redirect_uri)
        .map_err(|e| format!("ChatGPT did not complete the sign-in: {}. Run `rak login chatgpt` again", e.message))?;
    let id_token = tokens.id_token.clone().ok_or("ChatGPT sent no ID token; not signed in")?;
    let identity = auth::validate_id_token(&id_token, &auth::fetch_keys()?, &client_id, &nonce, auth::now())?;
    if account.subject.as_ref().is_some_and(|subject| *subject != identity.subject) {
        return Err(format!(
            "that is another account than {} (the one rak is registered for). For a different account or workspace: rak login chatgpt --new",
            account.who()
        ));
    }
    account.client_id = Some(client_id);
    account.subject = Some(identity.subject);
    account.email = identity.email.or(account.email);
    account.take(tokens, auth::now());
    auth::save(&file, &account)?;
    if !account.plan_allowed() {
        return Err(format!(
            "signed in as {}, but using your ChatGPT plan was not allowed, so rak can't send requests with it. \
             Run `rak login chatgpt` again and allow \"Use your ChatGPT plan\", or use a provider with an API key (`rak setup`)",
            account.who()
        ));
    }
    Ok(account)
}

/// Pure — unit-tested. The client id to exchange the code with: the saved
/// one, or the one this registration was just issued. A callback naming a
/// different client than the saved one is refused, never taken over.
pub fn issued_client(saved: Option<&str>, given: Option<&str>) -> Result<String, String> {
    match (saved, given) {
        (Some(saved), Some(given)) if saved != given => {
            Err("the sign-in answer names another client than the one rak is registered as; nothing was changed".to_string())
        }
        (Some(saved), _) => Ok(saved.to_string()),
        (None, Some(given)) if given != auth::REGISTRATION_CLIENT => Ok(given.to_string()),
        (None, _) => Err("the registration did not finish: ChatGPT issued no client id. Run `rak login chatgpt` again".to_string()),
    }
}

/// Signs out: the session is revoked at OpenAI, then the tokens are removed
/// here. The registration stays, for a later sign-in to the same account.
pub fn sign_out() -> Result<String, String> {
    let file = account_file()?;
    let mut account = auth::load(&file)?;
    if !account.signed_in() {
        return Ok("Not signed in to ChatGPT; nothing to do.".to_string());
    }
    let revoked = match (&account.client_id, &account.refresh_token) {
        (Some(client_id), Some(refresh_token)) => auth::revoke(client_id, refresh_token),
        _ => Ok(()),
    };
    account.clear_tokens();
    auth::save(&file, &account)?;
    Ok(match revoked {
        Ok(()) => format!("Signed out of ChatGPT ({}).", account.who()),
        Err(why) => format!(
            "Signed out here ({}), but OpenAI did not confirm ending the session ({why}). \
             To be sure, disconnect rak in ChatGPT's settings.",
            account.who()
        ),
    })
}

/// 127.0.0.1 only (never `localhost`): port 1455, or any free one.
fn listen() -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| format!("could not listen on 127.0.0.1 for the browser's answer: {e}"))
}

fn open_browser(url: &str) {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Waits for the browser to come to the callback, answers it with a short
/// page, and returns what it brought. Other requests (a favicon) get a 404.
fn wait_for_callback(listener: &TcpListener, state: &str) -> Result<auth::Callback, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + WAIT;
    loop {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err(
                        "the browser did not come back within 5 minutes; nothing was changed. Run `rak login chatgpt` again".to_string()
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(format!("waiting for the browser failed: {e}")),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 65_536 {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => request.extend_from_slice(&buffer[..n]),
            }
        }
        let Some(query) = callback_query(&String::from_utf8_lossy(&request)) else {
            respond(&mut stream, "404 Not Found", "Not found.");
            continue;
        };
        let outcome = auth::read_callback(&query, state);
        match &outcome {
            Ok(_) => respond(&mut stream, "200 OK", "You are signed in. Close this tab and go back to rak in your terminal."),
            Err(_) => respond(&mut stream, "400 Bad Request", "The sign-in did not finish. See rak in your terminal."),
        }
        return outcome;
    }
}

/// Pure — unit-tested. The query of a request to the callback path; None
/// for any other request.
pub fn callback_query(request: &str) -> Option<String> {
    let mut words = request.lines().next()?.split(' ');
    let (method, target) = (words.next()?, words.next()?);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (method == "GET" && path == PATH).then(|| query.to_string())
}

fn respond(stream: &mut std::net::TcpStream, status: &str, text: &str) {
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>rak</title>\
         <body style=\"font-family: system-ui, sans-serif; margin: 4rem auto; max-width: 32rem\"><h1>rak</h1><p>{text}</p></body>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_get_to_the_callback_path_is_the_answer() {
        assert_eq!(
            callback_query("GET /auth/callback?code=a&state=s HTTP/1.1\r\nHost: 127.0.0.1:1455\r\n\r\n").as_deref(),
            Some("code=a&state=s")
        );
        assert_eq!(callback_query("GET /auth/callback HTTP/1.1\r\n\r\n").as_deref(), Some(""));
        assert_eq!(callback_query("GET /favicon.ico HTTP/1.1\r\n\r\n"), None);
        assert_eq!(callback_query("GET /callback?code=a HTTP/1.1\r\n\r\n"), None, "/callback is not /auth/callback");
        assert_eq!(callback_query("POST /auth/callback?code=a HTTP/1.1\r\n\r\n"), None);
        assert_eq!(callback_query(""), None);
    }

    #[test]
    fn the_service_to_sign_in_to_is_named() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(after_name("login", &args(&["chatgpt"])).unwrap(), Vec::<&str>::new());
        assert_eq!(after_name("login", &args(&["chatgpt", "--status"])).unwrap(), ["--status"]);
        assert_eq!(after_name("login", &args(&["--help"])).unwrap(), ["--help"]);
        assert!(after_name("login", &[]).unwrap_err().starts_with("say what to sign in to: rak login chatgpt."));
        assert!(after_name("logout", &[]).unwrap_err().starts_with("say what to sign out of: rak logout chatgpt."));
        assert!(after_name("login", &args(&["--status"])).unwrap_err().contains("rak login chatgpt"));
        let claude = after_name("login", &args(&["claude"])).unwrap_err();
        assert!(claude.contains("does not know `claude`") && claude.contains("rak keys set NAME"));
    }

    #[test]
    fn the_client_id_comes_from_the_registration_and_is_never_swapped() {
        assert_eq!(issued_client(None, Some("oaiapp_1")).unwrap(), "oaiapp_1");
        assert_eq!(issued_client(Some("oaiapp_1"), None).unwrap(), "oaiapp_1", "a later sign-in may leave it out");
        assert_eq!(issued_client(Some("oaiapp_1"), Some("oaiapp_1")).unwrap(), "oaiapp_1");
        assert!(issued_client(Some("oaiapp_1"), Some("oaiapp_2")).unwrap_err().contains("another client"));
        assert!(issued_client(None, None).unwrap_err().contains("did not finish"));
        assert!(
            issued_client(None, Some("dynamic_agent_client")).unwrap_err().contains("did not finish"),
            "the entry point is not an id to keep"
        );
    }

    #[test]
    fn the_browser_s_answer_is_taken_and_answered_with_a_page() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let browser = std::thread::spawn(move || {
            let get = |target: &str| {
                let mut stream = std::net::TcpStream::connect(address).unwrap();
                write!(stream, "GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
                let mut answer = String::new();
                stream.read_to_string(&mut answer).unwrap();
                answer
            };
            (get("/favicon.ico"), get("/auth/callback?code=abc&state=st&client_id=oaiapp_7"))
        });
        let callback = wait_for_callback(&listener, "st").unwrap();
        assert_eq!(callback, auth::Callback { code: "abc".into(), client_id: Some("oaiapp_7".into()) });
        let (favicon, page) = browser.join().unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(page.starts_with("HTTP/1.1 200 OK") && page.contains("You are signed in"));
    }

    #[test]
    fn the_welcome_says_the_plan_is_in_use_and_where_to_manage_it() {
        let account = Account { email: Some("a@example.com".into()), ..Default::default() };
        let text = welcome(&account);
        assert!(text.starts_with("Signed in as a@example.com.") && text.contains("You're using your ChatGPT plan"));
        assert!(text.contains("Manage usage: https://chatgpt.com/settings/usage") && text.contains("--provider chatgpt"));
    }
}
