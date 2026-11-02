Generate `Containerfile` and download the required `setup_bash.sh` file with:

```bash
yard build .
```

To build the image automatically, uncomment the Docker post-hook in `yard.yaml`, or run:

```bash
docker build . -t python-with-cuda
```
