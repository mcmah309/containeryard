```yaml name=config
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "Python dependencies built with named fragments"
```
```containerfile name=build
# Build dependencies in a separate Containerfile stage.
FROM python:3.11-slim AS builder

RUN python -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"

RUN pip install --no-cache-dir numpy pandas scipy
```
```containerfile name=main
# Copy the environment into the current stage where the module is declared.
COPY --from=builder /opt/venv /opt/venv

ENV PATH="/opt/venv/bin:$PATH"
```
```containerfile name=finalize
# Run after all declared modules.
RUN echo finalized
```
