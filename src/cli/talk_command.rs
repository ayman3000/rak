//! `rak talk log|who|say|clear`: the person's side of a room of agents
//! started with `--talk`. The room itself is in the library (rak::talk).

use rak::talk::{Member, Present, Room, EVERYONE, HUMAN};
use std::io::{IsTerminal, Read, Seek, SeekFrom};
use std::path::PathBuf;

pub const TALK_HELP: &str = "\
rak talk — agents in one folder, messaging each other

Start one rak session per agent, in the same folder, each in its own terminal:

  rak --talk --name A -m MODEL
  rak --talk --name B -m OTHER-MODEL --role \"reviewer: tests the code, edits nothing\"

Each agent gets two tools, send_message and wait_message, and is told who else
is in the room. Give each its own request as usual. A message from another
agent is never taken as yours: each agent still asks you before it edits or
runs anything.

  rak talk log               the conversation so far, then each new message as
                             it comes (Ctrl-C to stop; --no-follow: print and exit)
  rak talk who               who is in the room now
  rak talk say \"TEXT\"        a message from you, to everyone
  rak talk say --to B \"TEXT\" the same, to one agent
  rak talk clear             set the conversation aside and start an empty room

OPTIONS
  -C, --cwd DIR              the project folder (default: where you are)

The room is the folder .rak/talk in the project: one small file per message,
and talk.md, the conversation as text. A session started when nobody else is
there begins a new conversation (the old one is kept under .rak/talk/archive);
rak -c --talk goes on with the last one.
";

/// Pure — unit-tested. Whether `rak talk …` is one of the commands here, and
/// not a request that happens to begin with the word "talk".
pub fn is_command(args: &[String]) -> bool {
    // `rak talk -C DIR log`: the folder comes before the word.
    let mut words: Vec<&str> = Vec::new();
    let mut rest = args.iter().map(String::as_str);
    while let Some(arg) = rest.next() {
        match arg {
            "-C" | "--cwd" => {
                rest.next();
            }
            _ => words.push(arg),
        }
    }
    // One word after `talk` is a command, known or mistyped: a mistyped one
    // gets an error, not a model call.
    words.len() <= 1 || matches!(words[0], "log" | "who" | "say" | "clear" | "help" | "-h" | "--help")
}

/// "A · with B (deepseek, reviewer)", for the welcome panel.
pub fn room_short(member: &Member) -> String {
    let others = member.others();
    if others.is_empty() {
        format!("you are {} · nobody else yet", member.me.name)
    } else {
        format!("you are {} · with {}", member.me.name, others.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", "))
    }
}

/// One line saying who this session is in the room and who else is there.
pub fn room_line(member: &Member) -> String {
    let others = member.others();
    let who =
        if others.is_empty() { "nobody else yet".to_string() } else { others.iter().map(Present::label).collect::<Vec<_>>().join(", ") };
    format!("talk: this session is {} in this folder's room · also there: {who}", member.me.name)
}

fn fail(message: &str) -> ! {
    eprintln!("rak talk: {message}\nTry `rak talk --help`.");
    std::process::exit(2);
}

pub fn run(args: &[String]) -> ! {
    let mut cwd: Option<PathBuf> = None;
    let mut words: Vec<String> = Vec::new();
    let mut to: Option<String> = None;
    let mut follow = true;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-C" | "--cwd" => cwd = Some(PathBuf::from(rest.next().unwrap_or_else(|| fail("--cwd needs a folder")))),
            "--to" => to = Some(rest.next().unwrap_or_else(|| fail("--to needs a name")).clone()),
            "--no-follow" => follow = false,
            "-h" | "--help" | "help" => {
                print!("{TALK_HELP}");
                std::process::exit(0);
            }
            _ => words.push(arg.clone()),
        }
    }
    let project = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !project.is_dir() {
        fail(&format!("{} is not a folder", project.display()));
    }
    let room = Room::in_project(&project);
    match words.first().map(String::as_str) {
        None => {
            print!("{TALK_HELP}");
            std::process::exit(0);
        }
        Some("who") => {
            let present = room.present();
            if present.is_empty() {
                println!("Nobody is in this folder's room. Start an agent with: rak --talk --name NAME");
            }
            for agent in present {
                println!("{}", agent.label());
            }
        }
        Some("say") => {
            let text = words[1..].join(" ");
            if text.trim().is_empty() {
                fail("say what? rak talk say \"TEXT\"");
            }
            let present = room.present();
            // A session that starts in an empty room begins a new conversation,
            // so a word left now would be set aside unread.
            if present.is_empty() {
                fail("nobody is in the room, so nobody would read it. Start an agent first: rak --talk --name NAME");
            }
            let to = match to {
                None => EVERYONE.to_string(),
                Some(name) if name.eq_ignore_ascii_case(EVERYONE) => EVERYONE.to_string(),
                Some(name) => match present.iter().find(|p| p.name.eq_ignore_ascii_case(&name)) {
                    Some(found) => found.name.clone(),
                    None => {
                        let who = if present.is_empty() {
                            "nobody".to_string()
                        } else {
                            present.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")
                        };
                        fail(&format!("{name} is not in the room (in the room: {who})"))
                    }
                },
            };
            match room.send(HUMAN, &to, text.trim(), false) {
                Ok(n) => {
                    let whom = if to == EVERYONE { "everyone".to_string() } else { to };
                    println!("Sent as message {n} to {whom}. An agent reads it the next time it waits for messages.");
                }
                Err(why) => fail(&format!("the message could not be written: {why}")),
            }
        }
        Some("clear") => {
            let present = room.present();
            if !present.is_empty() {
                let who = present.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ");
                fail(&format!(
                    "{who} {} in the room. Stop those sessions first, then clear.",
                    if present.len() == 1 { "is" } else { "are" }
                ));
            }
            match room.archive() {
                Ok(true) => println!("The conversation is kept under {} and the room is empty.", room.dir().join("archive").display()),
                Ok(false) => println!("Nothing to clear: this folder's room has no conversation."),
                Err(why) => fail(&format!("the conversation could not be moved: {why}")),
            }
        }
        Some("log") => log(&room, follow && std::io::stdout().is_terminal()),
        Some(other) => fail(&format!("there is no `{other}`: the commands are log, who, say and clear")),
    }
    std::process::exit(0);
}

/// Prints talk.md, then (when following) each addition as it comes.
fn log(room: &Room, follow: bool) {
    let file = room.log_file();
    let mut at: u64 = 0;
    let mut said_waiting = false;
    loop {
        match std::fs::File::open(&file) {
            Ok(mut handle) => {
                let size = handle.metadata().map(|m| m.len()).unwrap_or(0);
                // A new conversation began: the file is shorter than what was shown.
                if size < at {
                    println!("\n— a new conversation —\n");
                    at = 0;
                }
                if size > at && handle.seek(SeekFrom::Start(at)).is_ok() {
                    let mut text = String::new();
                    if handle.read_to_string(&mut text).is_ok() {
                        print!("{text}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                        at += text.len() as u64;
                    }
                }
            }
            Err(_) => {
                if at > 0 {
                    println!("\n— a new conversation —\n");
                    at = 0;
                }
                if !follow {
                    println!("No conversation in this folder yet. Start an agent with: rak --talk --name NAME");
                } else if !said_waiting {
                    println!("No conversation in this folder yet. Waiting for the first message (Ctrl-C to stop)…");
                    said_waiting = true;
                }
            }
        }
        if !follow {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn talk_followed_by_a_command_is_a_command_and_anything_else_is_a_request() {
        for command in ["log", "who", "say hello there", "clear", "--help", "-C /tmp/x log", "log --no-follow", ""] {
            assert!(is_command(&words(command)), "`rak talk {command}` was not taken as a command");
        }
        for mistyped in ["status", "logs", "-C /tmp/x list"] {
            assert!(is_command(&words(mistyped)), "`rak talk {mistyped}` would have gone to the model");
        }
        for request in ["to me about this code", "about the log", "me through who calls this"] {
            assert!(!is_command(&words(request)), "`rak talk {request}` was taken as a command");
        }
    }

    #[test]
    fn the_help_names_every_command_and_both_tools() {
        for word in ["rak talk log", "rak talk who", "rak talk say", "rak talk clear", "--talk", "--name", "send_message", "wait_message"] {
            assert!(TALK_HELP.contains(word), "the help does not mention {word}");
        }
    }
}
