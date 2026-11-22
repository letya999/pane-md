# Security policy

Security fixes target the current `main` branch and the latest tagged release.

Report vulnerabilities through [GitHub private vulnerability reporting](https://github.com/letya999/pane-md/security/advisories/new). Include the affected version, a sanitized reproducer, expected versus actual behavior, and the impact. Do not post exploits, credentials, private documents, or user data in a public issue. If private reporting is unavailable, open a public issue asking for a private contact **without vulnerability details**.

Markdown files, links, images, Mermaid source, and MCP tool arguments are untrusted input. The GUI should never fetch a remote image without a user action. The MCP server uses local STDIO; stdout is reserved for protocol messages, and diagnostics belong on stderr. Contributors should keep these boundaries intact and report bypasses privately.

If a secret is exposed, revoke or rotate it first, then coordinate cleanup of the repository and history. Do not paste the secret into another report or test fixture.
