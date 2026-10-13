```yaml name=config
description: "A module with build and main fragments"
```
```containerfile name=build
FROM alpine AS metadata-builder
```
```containerfile name=main
COPY --from=metadata-builder /etc/alpine-release /tmp/alpine-release
```
