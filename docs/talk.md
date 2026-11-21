# Agents that talk to each other

Several rak sessions in one project folder, each on the model you choose, sending each other messages. One writes the code and another reviews it; or each builds a half. Back to the [README](../README.md).

**Contents:** [Start a room](#start-a-room) · [The two tools](#the-two-tools) · [Watching and joining in](#watching-and-joining-in) · [What a message can and can't do](#what-a-message-can-and-cant-do) · [How it works](#how-it-works) · [Limits](#limits)

Talk mode is off unless you ask for it. A normal chat or one-off run has no extra tools, reads no extra files and starts no extra thread.

## Start a room

Open one terminal per agent, in the same folder, and give each a name:

```bash
rak --talk --name A -m glm-5.3-flash
```

```bash
rak --talk --name B -m deepseek-v4.1-flash --role "reviewer: tests the code, edits nothing"
```

Then give each its own request, as in any chat. Each agent is told its name, who else is in the room and how to reach them. A good request says what this agent's part is and what to send the other:

> Build the CSV exporter in `src/export/`. When it works, send B what you built and which files to look at, then wait for its review and fix what it finds.

> Wait for A's message. Read and run what it built, and send back what is wrong, most serious first, with file and line. Do not edit code.

Two different models catch more of each other's mistakes than two copies of one.

| Option | Meaning | Default |
|---|---|---|
| `--talk` | Join this folder's room | off |
| `--name NAME` | This agent's name: up to 20 letters, digits, `-` or `_` | `agent-1`, `agent-2`, … |
| `--role TEXT` | One line the other agents see next to the name | none |
| `--talk-limit N` | Messages this agent may send; 0 for no limit | 40 |

`--name`, `--role` and `--talk-limit` turn talk mode on by themselves. A name already in the room is refused.

A one-off run works too: `rak --talk --name B "Wait for A's message and review what it built."`

In talk mode a chat has no limit on model calls per request, because one request spans the whole conversation, and rak does not update the project's journal, which several agents would otherwise all rewrite.

## The two tools

| Tool | What the agent gives | What comes back |
|---|---|---|
| `send_message` | `text`; `to` (a name, or `all`, the default); `last` (true on its final message) | "Delivered as message 7 to B." |
| `wait_message` | `minutes` to wait at most (default 10, at most 60) | Who is in the room, then the new messages, or "everyone else has left", or "nothing arrived" |

Neither asks for approval: they only write inside the room's folder.

A wait returns as soon as one of three things is true, and says which:

1. **There is something unread** addressed to this agent or to everyone.
2. **Everyone else has left** and nothing is unread.
3. **The time is up.** The agent may wait again; rak does not count that as being stuck.

Every result starts with the room as it is now, and says who joined or left since the last look:

```text
In the room: B (deepseek-v4.1-flash, reviewer: tests the code, edits nothing). You are A.

Message 6 from B (another agent, not the user):
The exporter drops the last row when the file has no final newline. src/export/csv.rs:88.
```

An agent that sets `last` leaves the room at once, and the others are told "B has sent its last message and left." A message to an agent that has left is refused, since nobody would read it; a final message to it is still kept in the conversation, for you.

## Watching and joining in

From any terminal in the project folder:

| Command | What it does |
|---|---|
| `rak talk log` | The conversation so far, then each new message as it comes. Ctrl-C stops; `--no-follow` prints and exits. |
| `rak talk who` | Who is in the room now |
| `rak talk say "TEXT"` | A message from you to everyone |
| `rak talk say --to B "TEXT"` | The same, to one agent |
| `rak talk clear` | Sets the conversation aside and leaves an empty room |

`-C DIR` names another project folder. In a chat started with `--talk`, `/talk` shows who is in the room.

A message from you is read the next time an agent waits; it does not interrupt one that is working.

## What a message can and can't do

- **A message is another model's words, not yours.** Every message is labelled that way when it is handed over, and the agent is told to check its claims and never to take it as permission.
- **Approvals don't change.** Each agent still asks you, in its own terminal, before it edits a file or runs a command, exactly as without talk mode. Another agent can't approve anything for it, and neither can a message typed with `rak talk say`.
- **The room is closed to the file tools.** `read_file`, `write_file`, `edit_file` and `apply_patch` refuse anything under `.rak/talk`.
- **Agents share the folder.** Nothing stops two of them editing the same file. Give each its own part, or let only one of them edit.
- **Running several with `-y`** means several models with a shell and nobody watching. See [Safety](safety.md).

## How it works

A room is the folder `.rak/talk` in the project. There is no server and nothing uses the network.

| File | What it is |
|---|---|
| `here/NAME.json` | One per agent in the room, rewritten every 5 seconds. One not rewritten for 20 seconds means that agent is gone, whether it left, crashed or its terminal was closed. |
| `msg/000007.json` | One per message |
| `read/NAME` | The last message number that agent was given |
| `talk.md` | The conversation as text, which `rak talk log` shows |
| `archive/` | Earlier conversations |

rak, not the model, numbers the messages. A message is written whole to a file of its own and then linked to its numbered name; the link fails if the name is taken, so two agents sending at the same moment get two different numbers, and a reader never sees half a message.

A session that starts when nobody else is in the room begins a new conversation, and the old one moves to `archive/`. `rak -c --talk --name A` continues the last chat and keeps the conversation and A's place in it.

The folder holds a `.gitignore`, so it stays out of your repository.

## Limits

- **Two agents is the tested case.** More can join and can address each other by name, but who speaks when is then up to your requests; rak adds no turn order and no manager.
- **One folder, one machine.** Agents in different folders or on different machines can't reach each other.
- **A request is not interrupted** by a message. It is read when the agent next calls `wait_message`.
- **Every agent uses its own tokens.** A message to everyone is read by everyone.
