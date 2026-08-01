# Control harnesses headlessly rather than automating their terminal UI

There are two ways to drive Claude Code: its documented streaming JSON
interface, or typing into the terminal UI a human would use. Omnigent
implements the latter, and it costs roughly 17,500 lines for a single harness —
verified paste-injection, `capture-pane` polling, submit retries, slash-command
escaping and `statusLine` scraping — all of which exist only because a program
is driving an interface built for a person. We drive harnesses through
stream-json, hooks and `--resume` instead.

## Consequences

- A human can watch any agent's event stream but cannot type into one. All
  direction flows through the lead.
- Driving a harness's terminal UI is a rejected approach, not a deferred one.
  Reintroducing it means reopening this decision, not merely finding time.
