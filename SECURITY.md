# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| latest (`main`) | ✅ |
| older releases | ❌ patch fixes go to `main` only |

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report security issues by emailing the maintainer directly (see the GitHub profile for contact details). Include:

- A description of the vulnerability and its potential impact.
- Steps to reproduce or a proof-of-concept.
- Any suggested mitigations if you have them.

You will receive an acknowledgement within 48 hours and a resolution timeline as soon as the issue is assessed. We will coordinate a public disclosure once a fix is available.

## Security model

Slug runs as the current user and reads the OS accessibility layer to extract semantic trees from native apps. Key points to understand:

**Local only by default.** The HTTP transport binds to `127.0.0.1` only. The MCP server rejects any request whose `Origin` or `Host` header is not loopback, blocking DNS-rebinding and CSRF attacks from browser pages.

**API keys never in config files.** Slug stores AI provider keys exclusively in environment variables or `~/.slug/secrets.env` (created with `0600` permissions). They are never written to `slug.toml` or any committed file.

**Approval gate for destructive actions.** The agent controller flags actions containing keywords like `buy`, `pay`, `delete`, `send`, and `order` as destructive and pauses for human confirmation via the dashboard before executing.

**Accessibility bus scope.** The slug-ui bus server accepts connections only on a local Unix socket (macOS/Linux) or a local named pipe (Windows). It is not exposed over the network.

## Known limitations

- Accessibility APIs expose the full semantic content of the screen to any process with the Accessibility entitlement. Grant the permission only to trusted binaries.
- The approval-gate keyword list covers common destructive actions but is not exhaustive. Review agent goals before running in unattended mode.
