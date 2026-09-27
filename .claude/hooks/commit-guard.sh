#!/bin/bash
# Stage 10's other half. Git runs .githooks/pre-commit for every commit
# however the command is spelled; this refuses the commands that would make
# it not run, or that write commits without running it at all.
#
# Flags are read from the command's words as the shell would split them, so a
# commit message that merely mentions `-n` or `--no-verify` is not a flag.
# A command the shell would run from inside another — `bash -c '…'`, `eval`,
# backticks, `$(…)` — is read the same way, as a command of its own. Where the
# words cannot be split — unbalanced quotes — it falls back to the plain text
# and errs toward refusing.
#
# The word splitting is Python's `shlex`, which is the shell's own grammar for
# this; `python3` is the one thing the hook needs beyond bash and jq, and
# without it every command takes the refusing text fallback.
cmd=$(jq -r '.tool_input.command // empty')

deny() {
  jq -n --arg r "$1" '{hookSpecificOutput: {hookEventName: "PreToolUse",
    permissionDecision: "deny", permissionDecisionReason: $r}}'
  exit 0
}

bypass="An agent's commit goes through the review loop and its gate (.claude/skills/review/SKILL.md); this command would skip the gate."

# The text with its quotes and backslashes taken out, as the shell would
# join `core.hooks''Path` or `.git/ho"oks"` back into one word.
unquoted=$(tr -d "\"'\\\\" <<<"$cmd")

# Redirecting the hooks or writing the pass record by hand: refused wherever
# the words appear, quoted or not, since `-c "core.hooksPath=…"` is quoted.
# Git config keys ignore case, so `core.hookspath` is the same setting.
if grep -Eiq -- 'core\.hookspath|\.git/hooks|\.git/aldwin-review' <<<"$unquoted"; then
  deny "$bypass"
fi
# An alias names a subcommand this guard cannot see through — defined with
# `-c alias.…` or saved with `git config alias.…` for a later command.
if grep -Eiq -- '(^|[^[:alnum:]_])alias\.' <<<"$unquoted"; then
  deny "$bypass (a git alias hides the subcommand)"
fi

# The gate knows an agent by AGENT, so a command that names it other than
# to read it — `$AGENT`, `${AGENT}` — is refused: the plain ways to clear,
# unset, un-export or reassign it (`unset`, `read`, `printf -v`, …) all name
# it. `env -i` (in any flag cluster or abbreviation), `env -` and `exec -c`
# clear it without naming it. This is not every spelling a shell allows, and
# is not meant to be: a command built to get past it is the deliberate act
# the gate exists to make visible (aldwin-review.md, Decision 16).
if grep -Eq -- '(^|[^[:alnum:]_${]|[^$]\{|^\{)AGENT([^[:alnum:]_]|$)' <<<"$unquoted" ||
  grep -Eq -- '(^|[^[:alnum:]_-])env[[:space:]]+((-u|--unset)[[:space:]]+[^[:space:]]+[[:space:]]+|-[^[:space:]]*[[:space:]]+)*(-[[:alnum:]]*i[[:alnum:]]*|--i[[:alnum:]-]*|-)([[:space:]]|$)|(^|[^[:alnum:]_-])exec[[:space:]]+(-[^[:space:]]*[[:space:]]+)*-[[:alnum:]]*c[[:alnum:]]*([[:space:]]|$)' <<<"$unquoted"; then
  deny "$bypass (it names AGENT other than to read it, or clears the environment, and AGENT is how the gate knows an agent's commit)"
fi

verdict=$(python3 - "$cmd" <<'PY'
import re, shlex, sys

cmd = sys.argv[1]
# Commands that write commits without running pre-commit at all.
BYPASSING = {"commit-tree", "cherry-pick", "revert", "rebase", "am"}
# `git` options that take the next word as their value.
# (`--exec-path` is not one: bare, it prints the path.) The same list as the
# git shim's in crates/cli/src/git_shim.rs.
GIT_VALUED = {"-C", "-c", "--git-dir", "--work-tree", "--namespace",
              "--config-env", "--super-prefix", "--attr-source"}
# `git commit` short options whose value is the rest of the cluster or the
# next word, and long options that take the next word.
COMMIT_SHORT_VALUED = set("mFCct")
# Shells whose `-c` takes a script that is itself a command.
SHELLS = {"bash", "sh", "zsh", "dash", "ksh"}
COMMIT_LONG_VALUED = {"--message", "--file", "--reuse-message", "--reedit-message",
                      "--author", "--date", "--cleanup", "--fixup", "--squash",
                      "--template", "--trailer", "--pathspec-from-file"}

def words(text):
    lexer = shlex.shlex(text, posix=True, punctuation_chars=";&|()\n")
    lexer.whitespace = " \t\r"
    lexer.whitespace_split = True
    return list(lexer)

def commands(tokens):
    current = []
    for token in tokens:
        if token and set(token) <= set(";&|()\n"):
            if current:
                yield current
            current = []
        else:
            current.append(token)
    if current:
        yield current

def skips_merge_hook(args):
    """`merge` and `pull` skip pre-merge-commit only with `--no-verify` (or
    an abbreviation git accepts); their `-n` is `--no-stat`."""
    for word in args:
        if word == "--":
            return False
        if len(word) >= len("--no-veri") and "--no-verify".startswith(word):
            return True
    return False

def commit_skips_hooks(args):
    i = 0
    while i < len(args):
        word = args[i]
        if word == "--":
            return False
        # git takes any unambiguous abbreviation of a long option, and
        # `--no-veri` is the shortest that is not also `--no-verbose`.
        if len(word) >= len("--no-veri") and "--no-verify".startswith(word):
            return True
        if word.startswith("--"):
            if "=" not in word and word in COMMIT_LONG_VALUED:
                i += 1
        elif word.startswith("-") and len(word) > 1:
            for j, flag in enumerate(word[1:]):
                if flag == "n":
                    return True
                if flag in COMMIT_SHORT_VALUED:
                    if j == len(word) - 2:
                        i += 1
                    break
        i += 1
    return False

def substitutions(text):
    """The commands backticks and `$(…)` would run, read off the raw text:
    the word split breaks a backticked command across words."""
    yield from re.findall(r"`([^`]*)`", text)
    start = text.find("$(")
    while start >= 0:
        depth, k = 1, start + 2
        while k < len(text) and depth:
            depth += text[k] == "("
            depth -= text[k] == ")"
            k += 1
        yield text[start + 2:k - 1]
        start = text.find("$(", start + 2)

def scripts(command):
    """The scripts a command hands to a shell: `eval`'s words, `sh -c`'s."""
    for i, word in enumerate(command):
        name = word.rsplit("/", 1)[-1]
        if name == "eval":
            yield " ".join(command[i + 1:])
        if name in SHELLS:
            rest = command[i + 1:]
            for j, flag in enumerate(rest):
                if flag.startswith("-") and not flag.startswith("--") and "c" in flag[1:]:
                    if j + 1 < len(rest):
                        yield rest[j + 1]
                    break

def verdict(text):
    for inner in substitutions(text):
        found = verdict(inner)
        if found != "ok":
            return found
    for command in commands(words(text)):
        for script in scripts(command):
            found = verdict(script)
            if found != "ok":
                return found
        for start, word in enumerate(command):
            if word.rsplit("/", 1)[-1] != "git":
                continue
            i = start + 1
            while i < len(command) and command[i].startswith("-"):
                i += 2 if command[i] in GIT_VALUED else 1
            if i >= len(command):
                continue
            sub = command[i]
            if sub in BYPASSING:
                return "bypassing"
            if sub == "commit" and commit_skips_hooks(command[i + 1:]):
                return "no-verify"
            if sub in ("merge", "pull") and skips_merge_hook(command[i + 1:]):
                return "no-verify"
            # A global option this list does not know would hide the real
            # subcommand behind its value. So a `commit`, `merge` or `pull`
            # anywhere after `git` has its flags read too: when unsure, refuse.
            rest = command[start + 1:]
            if "commit" in rest:
                after = rest[rest.index("commit") + 1:]
                if commit_skips_hooks(after):
                    return "no-verify"
            for verb in ("merge", "pull"):
                if verb in rest and skips_merge_hook(rest[rest.index(verb) + 1:]):
                    return "no-verify"
    return "ok"

try:
    print(verdict(cmd))
except ValueError:
    print("unsplittable")
PY
)

case "$verdict" in
  ok) ;;
  no-verify) deny "$bypass (--no-verify, or its short form -n)" ;;
  bypassing) deny "This git command writes commits without the pre-commit hook, so it would skip the review loop's gate. Make the change with the review loop and git commit, or ask the developer to run it." ;;
  *)
    # The words could not be split: refuse anything that looks like a way
    # around the hook, as the plain-text check always did.
    # `--no-veri` is the shortest abbreviation git takes for `--no-verify`.
    if grep -Eq -- '--no-veri|(^|[^[:alnum:]_-])git[[:space:]].*(commit-tree|cherry-pick|revert|rebase|[[:space:]]am([[:space:]]|$)|commit.*[[:space:]]-[[:alpha:]]*n)' <<<"$cmd"; then
      deny "$bypass (the command's quoting could not be read, so it is refused to be safe)"
    fi ;;
esac
exit 0
