```yaml
description: "A split module without a finalize fragment"
split: true
```
```dockerfile
FROM alpine AS metadata-builder
```
```dockerfile
COPY --from=metadata-builder /etc/alpine-release /tmp/alpine-release
```
