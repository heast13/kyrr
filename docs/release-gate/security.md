# Security (Sonnet)

Input: `git diff vW..HEAD` of both repositories, SECURITY.md, the whole code
for the items below.

1. Dependencies: the CI job "audit" (`cargo audit`) is green; every entry
   in `.cargo/audit.toml` still has a true reason (else FAIL); new
   dependencies since vW are named, maintained and needed.
2. Secrets: nothing secret in the diff or in files that ship (keys, tokens,
   personal paths); API keys of own servers are stored only where the code
   says and never logged or put into the state file / process arguments.
3. Network: what talks to the network, to where, and only when the user
   turned it on; own-server addresses limited as intended (home network for
   http); TLS verified; nothing leaves the computer without the user's choice.
4. Files: new files and folders get private modes where they hold user data
   (voice memos 0700/0600 on Linux); temp files cleaned up; no writes outside
   kyrr's own places.
5. Processes: no user text in command lines (other users see them); shell
   commands quoted; Omarchy plugin: no `curl | sh`, no unverified download
   running, no `sudo` (the store's security baseline).
6. Input from outside (state file, socket lines, server answers, model
   output): parsed defensively, size-limited, never executed; the socket is
   private to the user.
7. Windows: the APO filter DLL and the installer: signed-or-documented,
   no new admin actions without need.
8. Anything else in the diff that an attacker on the same machine or the
   home network could use.
