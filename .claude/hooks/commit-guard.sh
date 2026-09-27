#!/bin/bash
# Stage 10's other half. Git runs .githooks/pre-commit for every commit
# however the command is spelled; this refuses the commands that would make
# it not run, or that write commits without running it at all.
cmd=$(jq -r '.tool_input.command // empty')

deny() {
  jq -n --arg r "$1" '{hookSpecificOutput: {hookEventName: "PreToolUse",
    permissionDecision: "deny", permissionDecisionReason: $r}}'
  exit 0
}

bypass="An agent's commit goes through the review loop and its gate (.claude/skills/review/SKILL.md); this command would skip the gate."

# Skipping or redirecting the hooks, or writing the pass record by hand.
if grep -Eq -- '--no-verify|core\.hooksPath|\.git/hooks|\.git/aldwin-review' <<<"$cmd"; then
  deny "$bypass"
fi
# `git commit -n` is --no-verify's short form. Checked only as a flag token,
# so a message that merely mentions -n may need rewording.
if grep -Eq '(^|[^[:alnum:]_-])git[[:space:]]([^;&|]*[[:space:]])?commit([[:space:]][^;&|]*)?[[:space:]]-[[:alpha:]]*n[[:alpha:]]*([[:space:]]|$)' <<<"$cmd"; then
  deny "$bypass (-n is --no-verify; if this was message text, reword it)"
fi
# Commands that write commits without running pre-commit at all.
if grep -Eq '(^|[^[:alnum:]_-])git[[:space:]]([^;&|]*[[:space:]])?(commit-tree|cherry-pick|revert|rebase|am)([[:space:]]|$)' <<<"$cmd"; then
  deny "This git command writes commits without the pre-commit hook, so it would skip the review loop's gate. Make the change with the review loop and git commit, or ask the developer to run it."
fi
exit 0
