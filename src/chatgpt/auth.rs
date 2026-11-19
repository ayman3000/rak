//! Sign in with ChatGPT, the open-source route OpenAI documents at
//! developers.openai.com/siwc/token-sharing-open-source: an OAuth sign-in in
//! the browser (PKCE, a callback on 127.0.0.1), after which requests use the
//! person's ChatGPT plan instead of an API key.
//!
//! - **rak registers as itself.** The first sign-in goes in as
//!   `dynamic_agent_client` with the name "rak"; OpenAI gives back a client
//!   id of rak's own for that account, which is kept and used from then on.
//!   No other tool's identity is borrowed.
//! - **One file**, ~/.config/rak/chatgpt.json, owner-only: this computer's
//!   host id, the client id, who signed in, and the tokens. Signing out
//!   clears the tokens and keeps the registration.
//! - **Tokens.** The access token lasts an hour; the refresh token is
//!   replaced on every use, so a refresh is done under a file lock and the
//!   file is read again once the lock is held (two raks never spend the same
//!   refresh token).
//!
//! The terminal side (opening the browser, waiting for the callback) is in
//! src/cli/login.rs; this file is the parts with no terminal in them.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ISSUER: &str = "https://auth.openai.com";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";
/// What the tokens are for; sent on sign-in, code exchange and refresh.
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// Without this one a sign-in is good but pays for nothing.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// The entry point for a first sign-in; never the id to keep.
pub const REGISTRATION_CLIENT: &str = "dynamic_agent_client";
/// The name the consent page shows; the same on every installation.
pub const APP_NAME: &str = "rak";
/// Where a person sees and limits what rak uses of their plan.
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";
pub const NOT_SIGNED_IN: &str = "not signed in to ChatGPT: run `rak login chatgpt`";
/// A token this close to its end is renewed first.
const RENEW_BEFORE: u64 = 300;

/// What chatgpt.json holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Account {
    /// This computer, as OpenAI's `ext_agent_host_id`: made once, kept.
    pub host_id: String,
    /// rak's client id for this ChatGPT account, from the first sign-in.
    pub client_id: Option<String>,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub scopes: Vec<String>,
    /// When the access token ends, in Unix seconds.
    pub expires_at: u64,
}

impl Account {
    pub fn signed_in(&self) -> bool {
        self.access_token.is_some() && self.refresh_token.is_some()
    }

    /// Whether the sign-in lets requests use the ChatGPT plan.
    pub fn plan_allowed(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }

    /// For people: the email, or "your ChatGPT account".
    pub fn who(&self) -> String {
        self.email.clone().unwrap_or_else(|| "your ChatGPT account".to_string())
    }

    /// Pure — unit-tested.
    pub fn to_json(&self) -> Value {
        json!({
            "ext_agent_host_id": self.host_id,
            "client_id": self.client_id,
            "subject": self.subject,
            "email": self.email,
            "access_token": self.access_token,
            "refresh_token": self.refresh_token,
            "scopes": self.scopes,
            "expires_at": self.expires_at,
        })
    }

    /// Pure — unit-tested.
    pub fn from_json(v: &Value) -> Account {
        let text = |name: &str| v[name].as_str().filter(|s| !s.is_empty()).map(str::to_string);
        Account {
            host_id: text("ext_agent_host_id").unwrap_or_default(),
            client_id: text("client_id"),
            subject: text("subject"),
            email: text("email"),
            access_token: text("access_token"),
            refresh_token: text("refresh_token"),
            scopes: v["scopes"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect(),
            expires_at: v["expires_at"].as_u64().unwrap_or(0),
        }
    }

    /// Takes a token answer in: the new tokens, scopes and end time together.
    pub fn take(&mut self, tokens: Tokens, now: u64) {
        self.access_token = Some(tokens.access_token);
        // The refresh token is replaced on every use; an answer without one
        // (not expected) leaves the old one.
        if tokens.refresh_token.is_some() {
            self.refresh_token = tokens.refresh_token;
        }
        if !tokens.scopes.is_empty() {
            self.scopes = tokens.scopes;
        }
        self.expires_at = now + tokens.expires_in;
    }

    /// Signed out: the tokens go, the registration stays for the next sign-in.
    pub fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.expires_at = 0;
    }
}

/// ~/.config/rak/chatgpt.json (or under XDG_CONFIG_HOME).
pub fn file() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config"))).map(|base| base.join("rak").join("chatgpt.json"))
}

/// The account in `file`; an empty one when there is no file yet.
pub fn load(file: &Path) -> Result<Account, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => {
            let value: Value = serde_json::from_str(&text)
                .map_err(|e| format!("{} is not readable ({e}); run `rak login chatgpt` again", file.display()))?;
            Ok(Account::from_json(&value))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Account::default()),
        Err(e) => Err(format!("could not read {}: {e}", file.display())),
    }
}

pub fn save(file: &Path, account: &Account) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&account.to_json()).unwrap_or_default() + "\n";
    crate::private_file::write(file, &text).map_err(|e| format!("could not write {}: {e}", file.display()))
}

/// Whether someone is signed in here, and as whom: for the provider lists.
pub fn status() -> Option<String> {
    let account = load(&file()?).ok()?;
    account.signed_in().then(|| account.who())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn http() -> ureq::Agent {
    crate::http::builder().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(60)).build()
}

// ---- random values, PKCE ----

fn random_bytes(n: usize) -> Vec<u8> {
    use ring::rand::SecureRandom;
    let mut bytes = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut bytes).expect("the system gives random bytes");
    bytes
}

/// A fresh random value for `state`, `nonce` or the PKCE verifier.
pub fn random_token() -> String {
    base64url(&random_bytes(32))
}

/// `urn:uuid:` and a random (version 4) UUID.
pub fn new_host_id() -> String {
    let mut b = random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("urn:uuid:{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// Pure — unit-tested. The PKCE challenge for a verifier: base64url of its
/// SHA-256, no padding.
pub fn challenge(verifier: &str) -> String {
    base64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref())
}

/// Pure — unit-tested. base64url without padding.
pub fn base64url(bytes: &[u8]) -> String {
    crate::text::base64(bytes).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

/// Pure — unit-tested. Reads base64url (or plain base64), padded or not.
pub fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u8);
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---- the sign-in address and its callback ----

/// One sign-in attempt, before the browser opens.
pub struct Attempt<'a> {
    /// The saved client id; None for a first sign-in (a registration).
    pub client_id: Option<&'a str>,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    pub verifier: &'a str,
    pub host_id: &'a str,
    /// The email signed in before, to pick the account for the person.
    pub login_hint: Option<&'a str>,
    /// Ask for the permissions again (after plan use was declined).
    pub consent: bool,
}

/// Pure — unit-tested. The address to open in the browser.
pub fn authorize_url(a: &Attempt) -> String {
    let code_challenge = challenge(a.verifier);
    let mut query: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", a.client_id.unwrap_or(REGISTRATION_CLIENT)),
        ("redirect_uri", a.redirect_uri),
        ("scope", SCOPES),
        ("resource", RESOURCE),
        ("state", a.state),
        ("nonce", a.nonce),
        ("code_challenge_method", "S256"),
        ("code_challenge", &code_challenge),
        ("ext_agent_host_id", a.host_id),
    ];
    match a.client_id {
        // The name goes only with a registration.
        None => query.push(("agent_name_hint", APP_NAME)),
        Some(_) => {
            if let Some(email) = a.login_hint {
                query.push(("login_hint", email));
            }
        }
    }
    if a.consent {
        query.push(("prompt", "consent"));
    }
    let query: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect();
    format!("{AUTHORIZE_URL}?{}", query.join("&"))
}

/// Pure — unit-tested.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pure — unit-tested. The names and values of a query string.
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    let decode = |text: &str| {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' => {
                    let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                    match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        Some(byte) => {
                            out.push(byte);
                            i += 2;
                        }
                        None => out.push(b'%'),
                    }
                }
                other => out.push(other),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

/// What the browser brought back to the callback.
#[derive(Debug, PartialEq)]
pub struct Callback {
    pub code: String,
    /// The client id OpenAI issued, on a registration.
    pub client_id: Option<String>,
}

/// Pure — unit-tested. Whether a request to the callback path is the answer
/// to this attempt: it carries this attempt's state. One with another state,
/// or none, is not, and the wait for the real answer goes on. The exception
/// is a refusal with no state at all, which is still reported, so a refused
/// sign-in never looks like a browser that did not come back.
pub fn answers(query: &str, state: &str) -> bool {
    let pairs = parse_query(query);
    let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
    match get("state") {
        Some(theirs) => theirs == state,
        None => get("error").is_some(),
    }
}

/// Pure — unit-tested. Reads the callback's query: the state must be this
/// attempt's, an error stops it, and a code is needed.
pub fn read_callback(query: &str, state: &str) -> Result<Callback, String> {
    let pairs = parse_query(query);
    let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    if get("state").as_deref() != Some(state) {
        return Err("the sign-in answer does not belong to this attempt (its state differs); nothing was changed".to_string());
    }
    if let Some(error) = get("error") {
        return Err(match error.as_str() {
            "access_denied" => "the sign-in was cancelled in the browser; nothing was changed".to_string(),
            _ => format!("ChatGPT refused the sign-in: {error}{}", get("error_description").map(|d| format!(" ({d})")).unwrap_or_default()),
        });
    }
    let code = get("code").ok_or("the sign-in answer carried no code")?;
    Ok(Callback { code, client_id: get("client_id") })
}

// ---- tokens ----

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub scopes: Vec<String>,
    pub expires_in: u64,
}

/// A refusal from the token address.
#[derive(Debug, PartialEq)]
pub struct TokenError {
    /// The machine-readable code ("invalid_grant"), when the answer had one.
    pub code: Option<String>,
    pub message: String,
}

impl TokenError {
    /// Pure — unit-tested. The refresh token is spent, expired or revoked:
    /// only a new sign-in helps.
    pub fn needs_sign_in(&self) -> bool {
        const ENDED: [&str; 6] = [
            "invalid_grant",
            "invalid_refresh_token",
            "token_expired",
            "refresh_token_expired",
            "refresh_token_invalidated",
            "refresh_token_reused",
        ];
        self.code.as_deref().is_some_and(|code| ENDED.contains(&code))
    }
}

/// Pure — unit-tested. A token answer's body.
pub fn read_tokens(body: &Value) -> Result<Tokens, TokenError> {
    let access_token =
        body["access_token"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| read_token_error(body, "no access token in the answer"))?;
    Ok(Tokens {
        access_token: access_token.to_string(),
        refresh_token: body["refresh_token"].as_str().map(str::to_string),
        id_token: body["id_token"].as_str().map(str::to_string),
        scopes: body["scope"].as_str().unwrap_or("").split_whitespace().map(str::to_string).collect(),
        expires_in: body["expires_in"].as_u64().unwrap_or(3600),
    })
}

/// Pure — unit-tested. The code and words of a refusal: OAuth's
/// `{"error": "invalid_grant", "error_description": …}` or the API's
/// `{"error": {"code": …, "message": …}}`.
pub fn read_token_error(body: &Value, otherwise: &str) -> TokenError {
    let error = &body["error"];
    let (code, message) = match error {
        Value::String(code) => (Some(code.clone()), body["error_description"].as_str().unwrap_or(code).to_string()),
        Value::Object(_) => {
            (error["code"].as_str().map(str::to_string), error["message"].as_str().map(str::to_string).unwrap_or_else(|| error.to_string()))
        }
        _ => (None, body["detail"].as_str().unwrap_or(otherwise).to_string()),
    };
    TokenError { code, message: crate::text::one_line(&message, 300) }
}

fn token_request(url: &str, form: &[(&str, &str)]) -> Result<Tokens, TokenError> {
    match http().post(url).send_form(form) {
        Ok(response) => {
            let body: Value =
                response.into_json().map_err(|e| TokenError { code: None, message: format!("unreadable answer from {url}: {e}") })?;
            read_tokens(&body)
        }
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
            let mut error = read_token_error(&body, &crate::text::one_line(&text, 300));
            error.message = format!("{url} answered {code}: {}", error.message);
            Err(error)
        }
        Err(ureq::Error::Transport(t)) => Err(TokenError { code: None, message: format!("could not reach {url}: {t}") }),
    }
}

/// The sign-in's code for tokens. No client secret: rak is a public client.
pub fn exchange(client_id: &str, code: &str, verifier: &str, redirect_uri: &str) -> Result<Tokens, TokenError> {
    token_request(
        TOKEN_URL,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("resource", RESOURCE),
        ],
    )
}

fn refresh(client_id: &str, refresh_token: &str) -> Result<Tokens, TokenError> {
    token_request(
        TOKEN_URL,
        &[("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token), ("resource", RESOURCE)],
    )
}

/// Held while a refresh is done, so two raks don't spend one refresh token.
struct Lock(#[allow(dead_code)] std::fs::File);

impl Lock {
    fn take(account_file: &Path) -> Option<Lock> {
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(account_file.with_extension("lock")).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // Blocks until the other rak is done; released when the file closes.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        }
        Some(Lock(file))
    }
}

/// The access token to send, renewed first when it is about to end.
/// `refused`: a token the API just answered 401 to; one is fetched anew
/// unless another rak already has.
pub fn access_token(file: &Path, refused: Option<&str>) -> Result<String, String> {
    let usable = |account: &Account| {
        let token = account.access_token.as_deref()?;
        (account.expires_at > now() + RENEW_BEFORE && refused != Some(token)).then(|| token.to_string())
    };
    let account = load(file)?;
    if !account.signed_in() {
        return Err(NOT_SIGNED_IN.to_string());
    }
    if !account.plan_allowed() {
        return Err(
            "this ChatGPT sign-in does not allow using the plan: run `rak login chatgpt` and allow \"Use your ChatGPT plan\"".to_string()
        );
    }
    if let Some(token) = usable(&account) {
        return Ok(token);
    }
    let _lock = Lock::take(file);
    // Another rak may have renewed it while this one waited for the lock.
    let mut account = load(file)?;
    if let Some(token) = usable(&account) {
        return Ok(token);
    }
    let (Some(client_id), Some(refresh_token)) = (account.client_id.clone(), account.refresh_token.clone()) else {
        return Err(NOT_SIGNED_IN.to_string());
    };
    match refresh(&client_id, &refresh_token) {
        Ok(tokens) => {
            account.take(tokens, now());
            save(file, &account)?;
            Ok(account.access_token.unwrap_or_default())
        }
        Err(error) if error.needs_sign_in() => {
            account.clear_tokens();
            let _ = save(file, &account);
            Err("your ChatGPT sign-in has ended (or rak was disconnected in ChatGPT's settings): run `rak login chatgpt`".to_string())
        }
        // A network failure is no reason to throw the sign-in away: a token
        // that has not ended yet is still used.
        Err(error) => match account.access_token.filter(|token| account.expires_at > now() && refused != Some(token.as_str())) {
            Some(token) => Ok(token),
            None => Err(format!("could not renew the ChatGPT sign-in: {}", error.message)),
        },
    }
}

// ---- the ID token ----

fn discovery() -> Result<Value, String> {
    http()
        .get(DISCOVERY_URL)
        .call()
        .map_err(|e| format!("could not read {DISCOVERY_URL}: {e}"))?
        .into_json()
        .map_err(|e| format!("unreadable answer from {DISCOVERY_URL}: {e}"))
}

/// OpenAI's published signing keys.
pub fn fetch_keys() -> Result<Value, String> {
    let url = discovery()?["jwks_uri"].as_str().unwrap_or("https://auth.openai.com/.well-known/jwks.json").to_string();
    if !url.starts_with("https://auth.openai.com/") {
        return Err(format!("OpenAI's signing keys are at an unexpected address ({url}); not signing in"));
    }
    http()
        .get(&url)
        .call()
        .map_err(|e| format!("could not read {url}: {e}"))?
        .into_json()
        .map_err(|e| format!("unreadable answer from {url}: {e}"))
}

/// Who an ID token says signed in.
#[derive(Debug, PartialEq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
}

/// Pure — unit-tested. Checks an ID token: signed by one of `keys`, issued
/// by OpenAI for this client, not ended, and carrying this attempt's nonce.
pub fn validate_id_token(token: &str, keys: &Value, client_id: &str, nonce: &str, now: u64) -> Result<Identity, String> {
    let bad = |why: &str| format!("the ID token from ChatGPT is not valid ({why}); not signed in");
    let parts: Vec<&str> = token.split('.').collect();
    let [header, payload, signature] = parts.as_slice() else { return Err(bad("not three parts")) };
    let json = |part: &str| base64url_decode(part).and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let header_json = json(header).ok_or_else(|| bad("unreadable header"))?;
    let claims = json(payload).ok_or_else(|| bad("unreadable claims"))?;
    let signature = base64url_decode(signature).ok_or_else(|| bad("unreadable signature"))?;
    let signed = format!("{header}.{payload}");

    let alg = header_json["alg"].as_str().unwrap_or("");
    let kid = header_json["kid"].as_str();
    let candidates: Vec<&Value> =
        keys["keys"].as_array().into_iter().flatten().filter(|key| kid.is_none() || key["kid"].as_str() == kid).collect();
    let part = |key: &Value, name: &str| key[name].as_str().and_then(base64url_decode);
    let verified = candidates.iter().any(|key| match alg {
        "RS256" => match (part(key, "n"), part(key, "e")) {
            (Some(n), Some(e)) => ring::signature::RsaPublicKeyComponents { n, e }
                .verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
                .is_ok(),
            _ => false,
        },
        "ES256" => match (part(key, "x"), part(key, "y")) {
            (Some(x), Some(y)) => {
                let point = [&[4u8][..], &x, &y].concat();
                ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, point)
                    .verify(signed.as_bytes(), &signature)
                    .is_ok()
            }
            _ => false,
        },
        _ => false,
    });
    if !matches!(alg, "RS256" | "ES256") {
        return Err(bad(&format!("signed with {alg}, which rak does not check")));
    }
    if !verified {
        return Err(bad("the signature does not match OpenAI's keys"));
    }
    if claims["iss"].as_str() != Some(ISSUER) {
        return Err(bad("another issuer"));
    }
    let for_this_client = match &claims["aud"] {
        Value::String(aud) => aud == client_id,
        Value::Array(list) => list.iter().any(|aud| aud.as_str() == Some(client_id)),
        _ => false,
    };
    if !for_this_client {
        return Err(bad("issued for another client"));
    }
    if claims["exp"].as_u64().unwrap_or(0) <= now {
        return Err(bad("it has ended"));
    }
    if claims["nonce"].as_str() != Some(nonce) {
        return Err(bad("it belongs to another attempt"));
    }
    let subject = claims["sub"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad("no subject"))?.to_string();
    Ok(Identity { subject, email: claims["email"].as_str().map(str::to_string) })
}

// ---- signing out ----

/// Ends the renewable session at OpenAI. Ok(()) is their "done" (also for a
/// token that was already invalid).
pub fn revoke(client_id: &str, refresh_token: &str) -> Result<(), String> {
    let url = discovery()?["revocation_endpoint"].as_str().map(str::to_string).ok_or("OpenAI names no revocation address")?;
    if !url.starts_with("https://auth.openai.com/") {
        return Err(format!("the revocation address is unexpected ({url})"));
    }
    http()
        .post(&url)
        .send_form(&[("token", refresh_token), ("token_type_hint", "refresh_token"), ("client_id", client_id)])
        .map(|_| ())
        .map_err(|e| format!("{url}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    #[test]
    fn base64url_goes_both_ways_and_the_pkce_challenge_is_the_standard_one() {
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foob", &[0xfb, 0xff, 0xfe, 0x00]] {
            let text = base64url(bytes);
            assert!(!text.contains(['+', '/', '=']), "{text}");
            assert_eq!(base64url_decode(&text).unwrap(), bytes);
        }
        assert_eq!(base64url_decode("Zm9vYg==").unwrap(), b"foob", "padding is fine");
        assert!(base64url_decode("a b").is_none());
        // RFC 7636, appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_ne!(random_token(), random_token());
        let host = new_host_id();
        assert!(host.starts_with("urn:uuid:") && host.len() == 45 && host.as_bytes()[23] == b'4', "{host}");
    }

    fn attempt<'a>(client_id: Option<&'a str>) -> Attempt<'a> {
        Attempt {
            client_id,
            redirect_uri: "http://127.0.0.1:1455/auth/callback",
            state: "st",
            nonce: "no",
            verifier: "ver",
            host_id: "urn:uuid:1",
            login_hint: Some("a+b@example.com"),
            consent: false,
        }
    }

    #[test]
    fn a_first_sign_in_registers_as_rak_and_a_later_one_uses_the_issued_id() {
        let first = authorize_url(&attempt(None));
        assert!(first.starts_with("https://auth.openai.com/api/accounts/authorize?response_type=code&client_id=dynamic_agent_client&"));
        assert!(first.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback&"));
        assert!(first.contains("&scope=openid%20profile%20email%20offline_access%20resource.invoke%20chatgpt.tokens.use.direct&"));
        assert!(first.contains("&resource=https%3A%2F%2Fapi.openai.com%2Fv1&") && first.contains("&code_challenge_method=S256&"));
        assert!(first.contains("&ext_agent_host_id=urn%3Auuid%3A1") && first.ends_with("&agent_name_hint=rak"));
        assert!(!first.contains("login_hint") && !first.contains("prompt="), "a registration names no account");

        let mut again = attempt(Some("oaiapp_1"));
        again.consent = true;
        let later = authorize_url(&again);
        assert!(later.contains("client_id=oaiapp_1&") && !later.contains("agent_name_hint"), "the name goes only with a registration");
        assert!(later.contains("&login_hint=a%2Bb%40example.com") && later.ends_with("&prompt=consent"));
    }

    #[test]
    fn the_callback_is_read_only_for_this_attempt() {
        let ok = read_callback("code=abc&scope=openid+email&state=st&client_id=oaiapp_9", "st").unwrap();
        assert_eq!(ok, Callback { code: "abc".into(), client_id: Some("oaiapp_9".into()) });
        assert_eq!(read_callback("code=a%2Fb&state=st", "st").unwrap(), Callback { code: "a/b".into(), client_id: None });
        assert!(read_callback("code=abc&state=other", "st").unwrap_err().contains("state"));
        assert!(read_callback("error=access_denied&state=st", "st").unwrap_err().contains("cancelled"));
        assert!(read_callback("error=server_error&error_description=try+later&state=st", "st")
            .unwrap_err()
            .contains("server_error (try later)"));
        assert!(read_callback("state=st", "st").unwrap_err().contains("no code"));
        // Whose answer a request is: this attempt's state, or a refusal that names none.
        assert!(answers("code=abc&state=st", "st") && answers("error=access_denied&state=st", "st"));
        assert!(!answers("code=abc&state=other", "st") && !answers("code=abc", "st") && !answers("", "st"));
        assert!(!answers("error=access_denied&state=other", "st"));
        assert!(answers("error=server_error", "st"), "a refusal with no state is still reported");
        assert_eq!(parse_query("a=1%zz&b"), [("a".to_string(), "1%zz".to_string()), ("b".to_string(), String::new())]);
    }

    #[test]
    fn a_token_answer_and_its_refusals_are_read() {
        let tokens = read_tokens(&json!({
            "access_token": "at", "refresh_token": "rt", "id_token": "it", "token_type": "Bearer", "expires_in": 3600,
            "scope": "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke"
        }))
        .unwrap();
        assert_eq!((tokens.access_token.as_str(), tokens.refresh_token.as_deref(), tokens.expires_in), ("at", Some("rt"), 3600));
        assert!(tokens.scopes.iter().any(|s| s == PLAN_SCOPE));

        let ended = read_token_error(&json!({ "error": "invalid_grant", "error_description": "refresh token reused" }), "");
        assert_eq!((ended.code.as_deref(), ended.message.as_str()), (Some("invalid_grant"), "refresh token reused"));
        assert!(ended.needs_sign_in());
        let reused = read_token_error(&json!({ "error": { "code": "refresh_token_reused", "message": "used before" } }), "");
        assert!(reused.needs_sign_in());
        let config = read_token_error(&json!({ "error": "invalid_client" }), "");
        assert!(!config.needs_sign_in(), "a wrong client is not a reason to sign in again");
        let plain = read_token_error(&Value::Null, "Bad Gateway");
        assert_eq!((plain.code, plain.message.as_str()), (None, "Bad Gateway"));
        assert!(read_tokens(&json!({ "error": "invalid_grant" })).unwrap_err().needs_sign_in());
    }

    #[test]
    fn the_account_file_is_owner_only_and_signing_out_keeps_the_registration() {
        let dir = temp_dir("chatgpt-account");
        let file = dir.join("chatgpt.json");
        assert_eq!(load(&file).unwrap(), Account::default(), "no file: nobody signed in");
        assert_eq!(access_token(&file, None).unwrap_err(), NOT_SIGNED_IN);

        let mut account = Account {
            host_id: "urn:uuid:1".into(),
            client_id: Some("oaiapp_1".into()),
            email: Some("a@example.com".into()),
            ..Default::default()
        };
        account.take(
            Tokens {
                access_token: "at".into(),
                refresh_token: Some("rt".into()),
                id_token: None,
                scopes: vec![PLAN_SCOPE.into()],
                expires_in: 3600,
            },
            now(),
        );
        save(&file, &account).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(load(&file).unwrap(), account);
        // A token with most of its hour left is used as it is: no request.
        assert_eq!(access_token(&file, None).unwrap(), "at");

        // A later answer without scopes or a refresh token keeps the old ones.
        account.take(Tokens { access_token: "at2".into(), refresh_token: None, id_token: None, scopes: vec![], expires_in: 60 }, 1000);
        assert_eq!((account.refresh_token.as_deref(), account.expires_at, account.plan_allowed()), (Some("rt"), 1060, true));

        account.clear_tokens();
        save(&file, &account).unwrap();
        let out = load(&file).unwrap();
        assert!(!out.signed_in());
        assert_eq!((out.client_id.as_deref(), out.host_id.as_str(), out.who().as_str()), (Some("oaiapp_1"), "urn:uuid:1", "a@example.com"));

        // Signed in, but plan use was not allowed: said so, not sent.
        account.take(
            Tokens {
                access_token: "at".into(),
                refresh_token: Some("rt".into()),
                id_token: None,
                scopes: vec!["openid".into()],
                expires_in: 3600,
            },
            now(),
        );
        save(&file, &account).unwrap();
        assert!(access_token(&file, None).unwrap_err().contains("Use your ChatGPT plan"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An ID token signed with a key made here (ES256), and that key's JWKS.
    fn signed_token(claims: &Value) -> (String, Value) {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let point = pair.public_key().as_ref();
        let keys = json!({ "keys": [
            { "kty": "EC", "kid": "other", "x": base64url(&[1; 32]), "y": base64url(&[2; 32]) },
            { "kty": "EC", "kid": "k1", "x": base64url(&point[1..33]), "y": base64url(&point[33..65]) }
        ]});
        let head = base64url(json!({ "alg": "ES256", "kid": "k1" }).to_string().as_bytes());
        let body = base64url(claims.to_string().as_bytes());
        let signature = pair.sign(&rng, format!("{head}.{body}").as_bytes()).unwrap();
        (format!("{head}.{body}.{}", base64url(signature.as_ref())), keys)
    }

    #[test]
    fn an_id_token_must_be_signed_by_openai_for_this_client_and_this_attempt() {
        let claims = json!({ "iss": ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "n1", "sub": "user-1", "email": "a@example.com" });
        let (token, keys) = signed_token(&claims);
        let who = validate_id_token(&token, &keys, "oaiapp_1", "n1", 1000).unwrap();
        assert_eq!(who, Identity { subject: "user-1".into(), email: Some("a@example.com".into()) });

        let fails = |token: &str, keys: &Value, client: &str, nonce: &str, now: u64| {
            validate_id_token(token, keys, client, nonce, now).unwrap_err()
        };
        assert!(fails(&token, &keys, "oaiapp_2", "n1", 1000).contains("another client"));
        assert!(fails(&token, &keys, "oaiapp_1", "n2", 1000).contains("another attempt"));
        assert!(fails(&token, &keys, "oaiapp_1", "n1", 2000).contains("ended"));
        // Claims changed after signing: the signature no longer matches.
        let parts: Vec<&str> = token.split('.').collect();
        let forged_claims = base64url(
            json!({ "iss": ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "n1", "sub": "someone-else" }).to_string().as_bytes(),
        );
        let forged = format!("{}.{forged_claims}.{}", parts[0], parts[2]);
        assert!(fails(&forged, &keys, "oaiapp_1", "n1", 1000).contains("signature"));
        // Signed by a key OpenAI does not publish.
        let (_, other_keys) = signed_token(&claims);
        assert!(fails(&token, &other_keys, "oaiapp_1", "n1", 1000).contains("signature"));
        // An unsigned token is never taken.
        let none = format!("{}.{}.", base64url(br#"{"alg":"none"}"#), parts[1]);
        assert!(fails(&none, &keys, "oaiapp_1", "n1", 1000).contains("none"));
        let (elsewhere, keys2) =
            signed_token(&json!({ "iss": "https://example.com", "aud": ["x", "oaiapp_1"], "exp": 2000, "nonce": "n1", "sub": "u" }));
        assert!(fails(&elsewhere, &keys2, "oaiapp_1", "n1", 1000).contains("issuer"));
        assert!(fails("abc", &keys, "oaiapp_1", "n1", 1000).contains("three parts"));
    }
}
