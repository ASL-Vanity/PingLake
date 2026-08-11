# Security Policy

## Supported version

Security fixes are applied to the latest released PingLake version.

## Boundaries

- The Agent is read-only and has no remote command endpoint.
- Agent credentials authenticate metrics ingestion only; they do not grant administrator access.
- Enrollment credentials should be rotated after expected nodes have enrolled.
- Internet-facing Hub deployments require HTTPS and a strong administrator password.
- Remote Agent binary downloads require an out-of-band SHA-256 value. Verify the release `SHA256SUMS.txt` before installation.
- Agent configuration, Hub environment files, SQLite data, and service logs must not be published.

## Reporting

Do not open a public issue containing credentials, database files, node identifiers, internal addresses, or webhook URLs. Provide a minimal reproduction with all sensitive values replaced.
