```bash
export UBUNTU_VERSION="24.04"
yard build .
podman build . -t simple
podman -it --rm simple
```