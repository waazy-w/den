# den

A clean, fast terminal UI for Claude Code, written in Rust. It drives your installed `claude` CLI
over its streaming JSON protocol, so your login, settings, MCP servers, skills, hooks and
permissions all work as usual.

## Install

You need [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and logged in
(`claude` on your PATH). Then, on macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/waazy-w/den/main/install.sh | sh
den            # restore saved sessions (or start one here)
den ~/code/api # also open a new session in that directory
```

Windows: run it inside [WSL](https://learn.microsoft.com/windows/wsl/install) (den needs a Unix terminal).

**Updates are automatic.** Every few hours den checks GitHub for a new release, installs it in the
background and tells you; `/reload` switches to it without stopping anything Claude is doing.
`/update` or `den update` checks right now. Turn it off with `"auto_update": false` in
`~/.config/den/config.json` or `DEN_NO_UPDATE=1`.

From source: `cargo install --path .`

`--no-mouse` turns off in-app mouse handling (native terminal selection, no wheel scroll). `DEN_CLAUDE_BIN` overrides
the `claude` binary path. Sessions are saved to `~/.config/den/sessions.json`; transcripts are
read back from Claude Code's own history, and a restored session resumes on your next message.

On first launch a short tour walks through the basics; `/tutorial` replays it.

## Projects and sessions

The sidebar groups sessions by project folder. `^N` opens a project picker listing the current
project, other open ones, and recent projects from your Claude Code history; type to filter, or
type a path (`~/code/x`, tab completes) to open any folder. `^P` jumps between sessions.
Status: spinner = working, ⚠ = needs your approval, ● = finished in the background.

Click **+ new session** in the sidebar (or `^N`) to pick a project, click the **+** beside a
project to add another session there, or type `/cd <path>`: folder names complete as you type
(tab descends, ⏎ opens). An untouched session just moves; otherwise a new one opens.

Folders of projects: `/workspace add ~/code` (or `/ws add …`) makes every sub-folder a project in
the `^N` picker; type a new name there to create the folder and start a session in it.
`/ws list`, `/ws remove <path>` manage them.

The bottom rows show context, tokens, time worked and cost, plus your plan usage windows
(5-hour and weekly) with time until reset.

Start a line with `!` to run it in your shell (in the session's folder): the output shows in the
conversation and is attached to your next message, so Claude sees it too.

**Token squeezer.** Build, install and test commands (`cargo`, `npm`, `pytest`, …) that your Claude
Code permission rules already allow are run through `den x`, which strips colors and progress bars,
collapses repeats and cuts huge logs to the start, the end and every error line (full output saved to
`~/.cache/den/out/`). Commands your rules don't cover are left alone, so you never get extra
permission prompts. `/stats` shows the savings; toggle it in settings. If you use RTK, den leaves
that job to RTK.

Long pastes collapse to `[Pasted text #1 +40 lines]` and images (drag a file in, or `^V` with a
screenshot on the clipboard) become `[Image #1]`; the full content is still sent. Messages typed
while Claude works are queued and sent when the turn ends.

## Keys

| key | action |
| --- | --- |
| `⏎` / `⇧⏎` `⌥⏎` `\⏎` | send / newline |
| `esc` | interrupt Claude, close popup |
| `y` `a` `n` | allow, always allow, deny a tool request |
| `^T` | browse tool calls; `⏎` expands, `esc` returns |
| `⇧tab` | cycle permission mode: default → accept edits → plan → auto |
| mouse drag | select text in the conversation; copied on release |
| `^O` | expand / collapse all tool calls |
| `PgUp` `PgDn` `⇧↑↓` wheel | scroll; `^L` back to bottom |
| `^N` / `^P` | new session (pick a project) / session switcher |
| `⌥1-9`, `⌥↑↓` | jump to / cycle sessions (needs Option-as-Meta on macOS) |
| `^D` (empty input) | close session |
| `^B` | toggle sidebar |
| `^F` | search every conversation; `⏎` jumps to the match |
| `^\` | split view: open a second pane / switch panes |
| `⌥M`, click the model name | model and thinking-effort picker |
| `^C ^C`, `^Q` | quit |
| `@` / `/` | file mention / command palette |

Hover a session in the sidebar and click its `×` to delete it (Claude Code keeps the transcript).
`/reload` restarts den in place (picking up a newly installed build) without interrupting Claude:
running `claude` processes keep working and the new den reattaches to their pipes, so replies keep
streaming, running tools finish, and pending approvals and queued messages carry over.
`/style` sets how much Claude explains; den sessions default to Claude Code's `default` style.

The sidebar shows what each session is doing (`editing fetch.rs`, `waiting for you`, `done · 2m ago`),
Claude's task list for the active session, and a `± N files` row that opens the files-changed view
with diffs (`/files`). When a session finishes or needs approval while you're in another session or
another app, macOS shows a silent notification (turn them off in settings, or `DEN_NO_NOTIFY=1`). Changing effort
restarts that session's `claude` (after the current reply) and keeps the conversation.

Commands: `/workspace add|remove|list <path>`, `/cd <path>`, `/reload`, `/style [name]`, `/new [path]`, `/close`, `/rename <name>`, `/clear`, `/model [name]`, `/effort [level]`, `/files`, `/search [text]`, `/mode [auto|plan|acceptEdits|default]`, `/stats`, `/copy`, `/tutorial`, `/help`, `/quit`.
Any other `/command` passes through to Claude Code.

## Themes and split view

`/theme` opens a picker with a live preview: dark (uses your terminal's background), midnight,
light, contrast and forest. Your pick is saved; first-time users choose one after the tour.

`^\` (or the **◫ split** button in the header, or `/split`) shows two sessions side by side. `^\`
again or a click moves between panes; each pane keeps its own draft. **×** closes a pane.

`^S` (or **settings** in the sidebar) toggles the typing effect (a soft cursor and fade-in while
Claude writes), notifications (silent, when Claude finishes or needs you) and auto-update.

When a usage window passes 80% (and again at 95%) den warns you, and the status bar shows when it resets.

## Releasing

Bump `version` in `Cargo.toml`, commit, then tag and push:

```sh
git tag v0.2.0 && git push origin main v0.2.0
```

GitHub Actions builds macOS (Apple Silicon, Intel) and Linux (x86_64, arm64) binaries and
publishes the release; everyone's den picks it up on its next check.

## Layout

`src/claude.rs` process + protocol I/O · `src/session.rs` event → transcript model ·
`src/ui/` rendering (cached per entry, only visible lines drawn) · `src/app.rs` keys and state.
