# Security Policy

## Reporting Vulnerabilities

Do not open public issues for security vulnerabilities.

Use GitHub private vulnerability reporting on this repository, or contact the maintainer directly through GitHub.

## Supported Versions

Only the latest released version is supported with security updates.

## Scope

`computer-use-linux` can observe and mutate the local Linux desktop through MCP, AT-SPI, portals, compositor APIs, and `ydotoold`. Security reports about unintended desktop access, tool annotation drift, unsafe defaults, packaging integrity, or release asset verification are in scope.

The binary opens no network listener unless `mcp --listen` is passed. That listener is plain HTTP behind a bearer token, a `Host` allow list and an `Origin` refusal. Reports about bypassing those checks are in scope; a deployment that exposes it without TLS in front is not.
