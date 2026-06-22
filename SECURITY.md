# Security Policy

## Supported Versions

Security fixes are only guaranteed for the latest released version of AlgoMLN.

Users are strongly encouraged to update to the newest release before reporting security issues.

| Version            | Supported      |
| ------------------ | -------------- |
| Development Builds | ✅ |

---

## Reporting a Vulnerability

If you discover a security vulnerability, please report it privately.

**Do not open a public GitHub issue.**

Public disclosure before a fix is available may put users at risk.

### Contact

Report vulnerabilities by:

* Opening a private GitHub Security Advisory (preferred)
* Contacting the maintainer directly at wtrmln_v1 on discord or codetest.reply@gmail.com

Include:

* Description of the issue
* Steps to reproduce
* Impact assessment
* Proof-of-concept (if available)
* Affected version(s)

---

## Security Scope

Examples of security issues include, but are not limited to:

### Credential Exposure

* Dhan access token leakage
* Environment variable exposure
* API credential disclosure
* Session token leakage

### Trade Execution Issues

* Unauthorized order placement
* Order manipulation
* Privilege escalation
* Strategy execution bypasses

### Data Exposure

* Sensitive user information disclosure
* Local storage vulnerabilities
* Configuration leakage
* Trading history exposure

### Remote Code Execution

* Arbitrary code execution
* Unsafe file handling
* Command injection
* IPC abuse

### Future Autonomous Trading Features

As autonomous trading capabilities are introduced, reports involving:

* Unintended order generation
* Strategy sandbox escapes
* Unauthorized broker actions
* Risk-control bypasses
* Position sizing failures
* Capital protection failures

will be treated as high-priority security issues.

---

## Out of Scope

The following generally do not qualify as security vulnerabilities:

* UI bugs
* Minor visual issues
* Typographical errors
* Feature requests
* Missing functionality
* Performance concerns without security impact

---

## Security Design Principles

AlgoMLN is designed around several security principles:

### Local First

User data should remain on the user's machine whenever possible.

### Explicit Broker Access

Broker integrations require explicit user configuration and authorization.

### No Hidden Execution Paths

Backtesting, paper trading, and live trading share the same strategy engine to reduce inconsistencies.

### Principle of Least Privilege

Components should only have access to the resources required for their operation.

---

## Responsible Disclosure

Please allow reasonable time for investigation and remediation before publicly disclosing a vulnerability.

Valid security reports will be acknowledged and investigated as quickly as possible.

Thank you for helping keep AlgoMLN secure.
