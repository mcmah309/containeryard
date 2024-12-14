```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "This is a modules description"
```
```dockerfile
RUN mkdir -p /app

WORKDIR /app
VOLUME /app

# Runs until the container is stopped
ENTRYPOINT ["/bin/sh", "-c"]
CMD ["tail -f /dev/null"]
```