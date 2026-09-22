Generate `Containerfile` and download the required `setup_bash.sh` file with:

```bash
yard build .
```

To build the image automatically, uncomment the Podman post-hook in `yard.yaml`, or run:

```bash
podman build . -t python-with-cuda
```
