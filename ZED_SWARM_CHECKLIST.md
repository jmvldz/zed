V Allow creating multiple agent panes
V Default to Claude Code or Codex
V Update history to store Claude Code sessions or Codex sessions

- Figure out how to update treatment of agent threads
	- Tab should use the icon of the agent that is being run
	- Agent title should update -> use any available LLM to summarize and create a short title for the thread
	- Move the new thread, history and settings -> move them into the tab menu on the right
- Default to Codex for new agents

So Zed is building support for loading session history with Claude Code and Codex but what that is, is a wrapper around the Claude Code SDK in the form of claude-code-acp. They didn't actually work directly with Anthropic, they just used the existing SDK.

Same for codex in codex-acp.