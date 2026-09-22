# ContainerYard

[<img alt="github" src="https://img.shields.io/badge/github-mcmah309/containeryard-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/mcmah309/containeryard)
[<img alt="crates.io" src="https://img.shields.io/crates/v/containeryard.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/containeryard)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-containeryard-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/containeryard)

ContainerYard is a declarative, reproducible, and reusable decentralized approach for defining containers. 
See [Why Use ContainerYard](#why-use-containeryard) for motivation.

ContainerYard breaks a containers definition into [modules](#module-files) and composes them with a [yard file](#yard-file). 

## Yard File

A yard file (`yard.yaml`) names the modules available to a project and composes them into one or more [Containerfiles](https://docs.docker.com/reference/dockerfile/). It has two required sections—`inputs` and `outputs`—plus optional build `hooks`.

### 1. Declare Inputs

Inputs give modules short names for use in outputs. Modules can come from local files or from a Git repository pinned to a commit:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-schema.json

inputs:
  modules:
    finalizer: local_modules/finalizer.md

  remotes:
    - url: https://github.com/mcmah309/yard_module_repository
      commit: 59e4aa77ee7e1c40adba40a7ab10e6b4fb9b8420
      modules:
        base: bases/ubuntu/lts.md
        git_config: dependent/git/git_config.md
        bash_flavor: dependent/apt/bash_interactive/flavors/mcmah309/mcmah309.md
```

The keys such as `base` and `git_config` are project-local names; the paths identify the module files at their source.

### 2. Compose Outputs

Each key under `outputs` is a file that `yard build` creates. Its entries are applied in order and may be named modules or inline Containerfile instructions:

```yaml
outputs:
  base.Containerfile:
    - base:
        version: "24.04"
    - RUN apt install --yes git
    - git_config:
        user_name: $(git config --get user.name)
        email: $GIT_EMAIL
```

Module arguments are nested beneath the module name. String arguments can use `$(command)` output or `$ENVIRONMENT_VARIABLE` values. A scalar such as `RUN apt install --yes git` is copied into the generated Containerfile as an inline instruction.

### 3. Reuse Outputs

An output can include another output by naming it with the same colon syntax used for a module:

```yaml
outputs:
  base.Containerfile:
    - base:
        version: "24.04"
    - git_config:
        user_name: $(git config --get user.name)
        email: $GIT_EMAIL

  development.Containerfile:
    - base.Containerfile:
    - bash_flavor:
    - finalizer:
```

ContainerYard expands `base.Containerfile` at that position before module validation and rendering. The generated `development.Containerfile` therefore contains `base`, `git_config`, `bash_flavor`, and `finalizer`; a module's `requires` can be satisfied by modules from the referenced output. This composes declarations—it does not add a `FROM` instruction—and both outputs are still generated.

### 4. Add Build Hooks (Optional)

Build hooks run commands around `yard build`:

```yaml
hooks:
  build:
    pre: yard update
    post: podman build -f development.Containerfile . -t my-image
```

The `pre` hook runs before module resolution, after which ContainerYard reloads `yard.yaml`. The `post` hook runs after all output files are written. Using `yard update` as the pre-hook keeps remote commits current.

Run `yard build` to generate every declared output. More complete examples are available in the [`examples` directory](https://github.com/mcmah309/containeryard/tree/master/examples).

## Module Files

Modules represent specific features of a container. e.g. The [rust module](https://github.com/mcmah309/yard_module_repository/blob/59e4aa77ee7e1c40adba40a7ab10e6b4fb9b8420/dependent/apt/rust/nightly.md) defines rust's installation. 
Modules can be easily reused, improved, and version controlled.

### Module File Format
A module consists of one file with an optional [config section](#configuration) and one or more [Containerfile (aka Dockerfile)](https://docs.docker.com/reference/dockerfile/) sections.

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

For [split modules](#split-modules), the file has a second Containerfile section and can optionally have a third.

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
# Template arguments accepted by this module
args:
  required:
  optional:
    - version
# Modules that must appear earlier in every output using this module.
# Paths are relative to this module file.
requires:
  - ../base/alpine.md
# Files to be pulled in with this module
required_files:
  - file/path
# Split this module across build, install, and optional finalize fragments
split: true
```
All of the above settings are optional

`requires` lists module files that must be included earlier in every output that uses the declaring module. Paths are resolved relative to the module file, and modules are matched by their source path rather than their `yard.yaml` input alias. For remote modules, the required module must come from the same repository and commit.

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

### Split Modules

Certain builders like Docker's BuildKit can parallelize stages for faster builds and smaller images. Split modules take advantage of this by dividing a module into two or three `containerfile`/`dockerfile` blocks: a required **build fragment**, a required **install fragment**, and an optional **finalize fragment**. Mark the module as split by setting `split: true` in its configuration block (it defaults to `false` when omitted).

When modules are combined, the **build fragments of all split modules are hoisted to the start** of the generated Containerfile, the **install fragment is injected where the module is declared**, and each optional **finalize fragment is appended after all declared modules**. Finalize fragments preserve module declaration order. This is useful for defining dependencies in isolated build stages (for example, a virtual environment built in a `builder` stage), copying the result into the final image, and deferring instructions that must run after the rest of the image is assembled.

For example, given a split module `python-deps` (`python_deps.md`):

````markdown
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "Python dependencies built in a split module"
split: true
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
```dockerfile
# Run after every declared module
RUN test -x /opt/venv/bin/python
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

# Run after every declared module
RUN test -x /opt/venv/bin/python
```

## Installation

Note: `yard` is the cli tool for ContainerYard.

### Debian - Ubuntu, Linux Mint, Pop!_OS, etc.

```bash
release_ver=<INSERT_CURRENT_VERSION> # e.g. release_ver='v0.4.0'
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

`yard build --with-cache-busting` transforms the generated Containerfile so that a cache-busting argument is inserted before each module fragment:

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

Repositories of module files. Good resource to see more examples and working configs for other users.

- <https://github.com/mcmah309/containeryard_modules.git> - mcmah309's Module Repository. Rust, Flutter, Bash, etc.

**\*Feel free to create a PR to add your own\***
