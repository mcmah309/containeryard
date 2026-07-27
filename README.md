# ContainerYard

[<img alt="github" src="https://img.shields.io/badge/github-mcmah309/containeryard-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/mcmah309/containeryard)
[<img alt="crates.io" src="https://img.shields.io/crates/v/containeryard.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/containeryard)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-containeryard-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/containeryard)

ContainerYard is a declarative, reproducible, and reusable decentralized approach for defining containers. 
See [Why Use ContainerYard](#why-use-containeryard) for motivation.

ContainerYard breaks a containers definition into [modules](#module-files) and composes them with a [yard file](#yard-file). 

## Yard File
A yard file (`yard.yaml`) composes [modules](#module-files) and outputs one or more Containerfiles (aka [Dockerfiles](https://docs.docker.com/reference/dockerfile/)). E.g.

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-schema.json

inputs:
  # Modules found on local paths
  modules:
    finalizer: local_modules/finalizer.md
  # Modules found in a remote repos
  remotes:
    - url: https://github.com/mcmah309/yard_module_repository
      commit: 59e4aa77ee7e1c40adba40a7ab10e6b4fb9b8420
      modules:
        base: bases/ubuntu/lts.md
        git_config: dependent/git/git_config.md
        bash_flavor: apt/bash_interactive/flavors/mcmah309/mcmah309.md

outputs:
  # Output Containerfile created from modules
  Containerfile:
    # Module "base" from inputs
    - base:
         # Inputs, shell commands `$(..)` and ENV vars `$..` also supported
        version: "24.04"
    # Inline module
    - RUN apt install git
    - git_config:
        user_name: $(git config --get user.name)
        email: $(git config --get user.email)
    - bash_flavor:
    - finalizer:

hooks:
  build:
    # Command executed before the build. Will reload this file after the command is executed
    pre: yard update
    post: podman build . -t git
```
Simply running `yard build` in the above case, will output a single Containerfile to your current directory.
See more `yard.yaml` examples [here](https://github.com/mcmah309/containeryard/tree/master/examples).

## Module Files

Modules represent specific features of a container. e.g. The [rust module](https://github.com/mcmah309/yard_module_repository/blob/59e4aa77ee7e1c40adba40a7ab10e6b4fb9b8420/dependent/apt/rust/nightly.md) defines rust's installation. 
Modules can be easily reused, improved, and version controlled.

### Module File Format
A module consists of one file with one to two parts - an optional [config section](#configuration) and a Containerfile [Containerfile (aka Dockerfile)](https://docs.docker.com/reference/dockerfile/))section.

````markdown
```yaml

# Configuration here

```
```dockerfile

# Dockerfile Statements Here

```
````

---
Alternatively the `yaml` configuration block can be omitted. Or if both the `yaml` and `dockerfile`/`containerfile` blocks are omitted, then the file is just interpreted as a regular Containerfile without any configuration (example [here](https://github.com/mcmah309/containeryard/blob/master/examples/local_python_dev_with_cuda/local.Containerfile)). 

For [independent modules](#independent-modules), the file has an additional Containerfile section.

### Module Parts

#### Containerfile
Containerfile defines the core of the module. E.g.
```dockerfile
# Note: `version` is defined in the configuration in the next section
FROM alpine:{{ version | default (value="latest") }}

RUN apk update \
    && apk upgrade \
    && apk add --no-cache ca-certificates \
    && update-ca-certificates
```
This file is first treated as a [Tera](https://keats.github.io/tera/docs/#templates) template, then compiled.
The result is a pure Containerfile component that can be combined with other modules.

#### Configuration

The configuration component is a `yaml` block and provides metadata for what the Containerfile component needs. E.g.
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "This is a modules description"
# Files to be pulled in with this module
args:
  required:
  optional:
    - version
# Files to be pulled in with this module
required_files:
  - file/path
# Is this module an independent module
independent: true
```
All of the above settings are optional

`yard.yaml` provides the values for `args:` declared in a this block.
e.g.
```yaml
inputs:
  modules:
    module: path/to/module

outputs:
  Containerfile:
    - module:
        version: "3.20.0"
```

### Putting It All Together

Combining the examples from the [Module Parts](#module-parts) section, the output of `yard build` would be
```dockerfile
FROM alpine:3.20.0

RUN apk update \
    && apk upgrade \
    && apk add --no-cache ca-certificates \
    && update-ca-certificates
```

For more module examples click [here](https://github.com/mcmah309/yard_module_repository/tree/master).

### Independent Modules

Certain builders like docker's buildkit can parallelize builds for faster building and smaller images. Independent modules take advantage of this. They have a **build stage** and an **install stage**, defined by two `containerfile`/`dockerfile` blocks in the module file. The first block is the build stage and the second block is the install stage. Mark the module as independent by setting `independent: true` in its configuration block (it defaults to `false` when omitted).

When modules are combined, the **build stages of all independent modules are hoisted to the start** of the generated Containerfile, and the **install stage is injected where the module is declared** in the `yard.yaml` outputs (like a normal module). This is useful for defining dependencies in isolated build stages (e.g. a virtual environment built in a `builder` stage) and copying the result into the final image.

For example, given an independent module `python-deps` (`python_deps.md`):

````markdown
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "Python dependencies installed in an independent build stage"
independent: true
```
```dockerfile
# Build & install dependencies
FROM python:3.11-slim AS python-deps-builder

RUN python -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"

RUN pip install --no-cache-dir numpy pandas scipy
```
```dockerfile
# Copy the entire virtual environment
COPY --from=python-deps-builder /opt/venv /opt/venv

ENV PATH="/opt/venv/bin:$PATH"
```
````

and a `yard.yaml`:

```yaml
inputs:
  modules:
    python-deps: python_deps.md

outputs:
  out.Containerfile:
    - FROM python:3.11-slim
    - RUN echo before
    - python-deps:
    - RUN echo after
```

the generated `out.Containerfile` would be:

```dockerfile
# Build & install dependencies
FROM python:3.11-slim AS python-deps-builder

RUN python -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"

RUN pip install --no-cache-dir numpy pandas scipy

FROM python:3.11-slim

RUN echo before

# Copy the entire virtual environment
COPY --from=python-deps-builder /opt/venv /opt/venv

ENV PATH="/opt/venv/bin:$PATH"

RUN echo after
```

## Installation

Note: `yard` is the cli tool for ContainerYard.

### Debian - Ubuntu, Linux Mint, Pop!_OS, etc.

```bash
release_ver=<INSERT_CURRENT_VERSION> # e.g. release_ver='v0.3.13'
deb_file="containeryard_$(echo $release_ver | sed 's/^v//')-1_amd64.deb"
curl -LO https://github.com/mcmah309/containeryard/releases/download/$release_ver/$deb_file
dpkg -i "$deb_file"
```

### Cargo

```bash
cargo install containeryard
```
Consider adding `--profile dist` for a longer compile time but a more optimal build.

## Extra

### Hooks

Hooks can be use in `yard.yaml` to run before or after `yard build`
```yaml
hooks:
  build:
    pre: yard update
    post: ./post_yard_build.sh

# ...
```
`yard update` is a good hook since it keeps versions up to date with any remotes.

### Yard Output

If you need the declared output filenames in order, `yard outputs` prints one output name per line. This is useful from scripts that want to process generated files in the same order as `yard.yaml`. e.g. in a `post_yard_build.sh` script:
```sh
#!/usr/bin/env bash
set -oeu pipefail

for item in $(yard outputs); do
    if [[ -f "$item" ]]; then
        # Strip '.Containerfile' from the end to use as the image tag
        tag="${item%.Containerfile}"
        
        echo "Building $tag from $item..."
        podman build -f "$item" -t "$tag"
    else
        echo "Error: File '$item' not found."
        exit 1
    fi
done
```

### Cache Busting

`yard build --with-cache-busting` transforms the generated Containerfile so that a cache busting argument is inserted between each module -

```dockerfile
ARG CACHE_BUST_<MODULE_NAME>=1
```

This lets you bust the build cache for specific modules using podman/docker's `--build-arg` flag, e.g.:

```bash
podman build --build-arg CACHE_BUST_RUST_ESSENTIALS=$(date +%s) -t my-app .
```

Module names come from the `yard.yaml` output declarations (e.g. `rust-essentials:`).

## FAQ
### Why Use ContainerYard?

Developers constantly rewrite the same Containerfile/Dockerfile configs. Besides taking away developer time, 
these configs become hard to maintain/upgrade and adding new features feels like starting from scratch again.
With ContainerYard, you can write your config once and easily reuse and incrementally improve it over time.
Users can then import these various modules with little to no configuration. Want Rust? Just add it to your `yard.yaml` file.
Want Flutter? Do the same. Need the latest version? Easily upgrade with `yard update` or just modify the commit line.
With ContainerYard you should never have to define certain Containerfile configs again. But
if you do want to do something custom, ContainerYard does not get in your way, everything is Containerfile based 
and the output is a pure Containerfile. No need to learn a complex tool, no need to re-invent the wheel, Containerfiles 
and Tera templates are powerful enough. Just let ContainerYard be the glue.

### Why Use ContainerYard Over Nix Flakes?

ContainerYard is heavily inspired by Nix flakes. In fact, ContainerYard can be thought of as Nix flakes meets Containerfiles (aka Dockerfiles).

Nix flakes guarantees reproducibility at the cost of developer flexibility. ContainerYard is decentralized, allowing users to easily use different package managers and upstreams. As such, ContainerYard sacrifices some reproducibility guarantees and gains complete developer flexibility.

ContainerYard is also extremely simple and built on familiar developer tools - Containerfiles and Tera templates.

## Contributing

Feel free to open an issue with any suggestions/ideas/bugs you may have and/or create PR's.

ContainerYard builds and uses its own dev container :D see [here](https://github.com/mcmah309/containeryard/tree/master/.devcontainer).
Open the project in vscode, click the "open in container" button and you are ready to go! Otherwise just use the provided Containerfile or your own local setup.


## Module Repositories

- <https://github.com/mcmah309/yard_module_repository.git> - mcmah309's Module Repository. Rust, Flutter, Bash, etc.

**\*Feel free to create a PR to add your own!\***

