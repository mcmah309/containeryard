```bash
yard build .
podman build . -t flutter_rust
podman -it --rm flutter_rust /bin/bash
```