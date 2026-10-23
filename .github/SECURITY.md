# Security policy

## Supported versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | ✅        |
| < 0.1   | ❌        |

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

Use GitHub's private vulnerability reporting:

<https://github.com/Linux-System-0/tarpack/security/advisories/new>

If that is unavailable, contact the maintainer
[@Linux-System-0](https://github.com/Linux-System-0) and ask for a private
channel.

Please include:

- the affected version or commit;
- operating system and filesystem type;
- a minimal reproduction, including the directory layout and commands;
- the impact, and whether it requires a malicious input tree, a malicious
  symlink, or a local attacker.

We aim to acknowledge a report within 7 days and provide a remediation plan
within 30 days. Please allow time for a fixed release before public disclosure.

## Scope

In scope:

- path traversal or writing outside the requested output path;
- bypassing the output containment check, including symlink or junction tricks;
- archive entries that can escape the extraction root when produced by
  `tarpack`;
- crashes or memory-safety issues reachable from an untrusted input tree.

Out of scope:

- vulnerabilities in dependencies (please report upstream; we track them with
  `cargo-audit`);
- behavior that intentionally follows a symlink with `-r/--recursive` to a
  location the user controls;
- issues that require root privileges or an already-compromised local account.

## Dependency advisories

`cargo-audit` runs on dependency changes and on a weekly schedule. Please open a
normal issue for known advisories that require a dependency bump.
