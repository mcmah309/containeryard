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
A Markdown module has an optional `yaml name=config` block and at least one named [Containerfile (aka Dockerfile)](https://docs.docker.com/reference/dockerfile/) fragment:

````markdown
```yaml name=config
# Configuration here
```
```dockerfile name=main
# Dockerfile Statements Here
```
````

Unnamed fences are documentation. `Containerfile`/`Dockerfile` inputs, including `.Containerfile`/`.Dockerfile` suffixes (case-insensitive), are treated as a single `main` fragment. Other files without Markdown fences work the same way.

`containerfile` and `dockerfile` are interchangeable. See [named fragments](#named-fragments) for placement rules.

### Module Parts

#### Containerfile
Containerfile defines the core of the module. E.g.
```dockerfile name=main
# Note: `version` is defined in the configuration in the next section
FROM alpine:{{ version | default (value="latest") }}

RUN apk update \
    && apk upgrade \
    && apk add --no-cache ca-certificates \
    && update-ca-certificates
```
This block is first treated as a [Tera](https://keats.github.io/tera/docs/#templates) template, then compiled.
The result is a pure Containerfile component that can be combined with other modules.

#### Configuration

The configuration component is a `yaml name=config` block and provides metadata for what the Containerfile component needs. E.g.
```yaml name=config
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
  - path: ../base/alpine.md
# Files to be pulled in with this module
files:
  - path: file/path
```
All of the above settings are optional

Use `args.validate` for relationships or constraints between arguments:

```yaml name=config
args:
  optional: [username, password]
  validate:
    - assert: "(username is defined) == (password is defined)"
      message: "username and password must be supplied together."
```

All assertions must pass after argument values are resolved and required/accepted arguments are checked. False assertions report their `message`; invalid expressions or undeclared argument references are configuration errors. Validation runs before module requirements and is unaffected by `--ignore-requires`.

`requires` checks that modules appear earlier in the output. Each entry needs a `path` relative to this module, matched by source file rather than input alias. Remote dependencies must share its repository and commit. Add `when` for a conditional requirement:

```yaml name=config
args:
  optional: [cuda]
requires:
  - path: ../base.md
  - path: ../cuda.md
    when: "cuda is defined and cuda == true"
```

`when` is a [Tera condition](https://keats.github.io/tera/#if) evaluated per output using declared, resolved module arguments. False skips the requirement. `is defined` includes supplied values of `false`, `0`, and `""`; use `== true` for an enabled boolean.

Requirements never add or reorder modules. `--ignore-requires` and `--ignore-all-requires` skip checks; configuration and paths are still validated.

`files` also uses `path` and optional `when`:

```yaml name=config
args:
  optional: [cuda]
files:
  - path: scripts/setup.sh
  - path: scripts/cuda.sh
    when: "cuda is defined and cuda == true"
```

Active files must exist locally or are fetched from the remote module's directory into the yard directory. False conditions skip fetching and existence checks; path syntax is always validated. Files are shared across outputs, so active declarations from different module sources cannot target the same destination. `--do-not-refetch` preserves existing remote files.

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

Combining the examples from the [Module Parts](#module-parts) section, the output of `yard build --no-cache-busting` would be
```dockerfile
FROM alpine:3.20.0

RUN apk update \
    && apk upgrade \
    && apk add --no-cache ca-certificates \
    && update-ca-certificates
```

For more module examples click [here](https://github.com/mcmah309/yard_module_repository/tree/master).

### Named Fragments

Name each `containerfile` or `dockerfile` block to select its placement:

| Fence | Placement |
|---|---|
| `containerfile name=build` | Before all `main` fragments |
| `containerfile name=main` | Where the module is declared in the output |
| `containerfile name=finalize` | After all `main` fragments |

Use any combination, once per name. Module declaration order is preserved within each group; Markdown block order does not affect placement. Unknown or duplicate names are errors. `FROM` instructions and stage names remain explicit.

For example, `python_deps.md`:

````markdown
```yaml name=config
description: "Python dependencies built in a separate stage"
```
```dockerfile name=build
FROM python:3.11-slim AS python-deps-builder
RUN python -m venv /opt/venv
RUN /opt/venv/bin/pip install --no-cache-dir numpy pandas scipy
```
```dockerfile name=main
COPY --from=python-deps-builder /opt/venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"
```
```dockerfile name=finalize
RUN test -x /opt/venv/bin/python
```
````

With this `yard.yaml`:

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

With `yard build --no-cache-busting`, the generated instructions are:

```dockerfile
FROM python:3.11-slim AS python-deps-builder
RUN python -m venv /opt/venv
RUN /opt/venv/bin/pip install --no-cache-dir numpy pandas scipy

FROM python:3.11-slim
RUN echo before

COPY --from=python-deps-builder /opt/venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"

RUN echo after
RUN test -x /opt/venv/bin/python
```

The build stage is hoisted, the copy stays at the module's declaration, and the final check runs after all `main` fragments.

## Installation

Note: `yard` is the cli tool for ContainerYard.

### Debian - Ubuntu, Linux Mint, Pop!_OS, etc.

Copy and paste the following to install the latest GitHub release. It automatically
selects the `.deb` for your architecture (`amd64` or `arm64`):

```bash
(
set -e
sudo apt-get update
sudo apt-get install -y curl
release_url=$(curl -fsSL -o /dev/null -w '%{url_effective}' https://github.com/mcmah309/containeryard/releases/latest)
tag=${release_url##*/}
deb="containeryard_${tag#v}-1_$(dpkg --print-architecture).deb"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT
curl -fL "https://github.com/mcmah309/containeryard/releases/download/$tag/$deb" -o "$tmp_dir/$deb"
sudo apt-get install -y "$tmp_dir/$deb"
yard --version
)
```

### Cargo

```bash
cargo install containeryard
```
Consider adding `--profile dist` for a longer compile time but a more optimal build.

### Nix

ContainerYard is available in [nixpkgs unstable](https://search.nixos.org/packages?channel=unstable&query=containeryard#show=containeryard).

```bash
nix profile install nixpkgs/nixos-unstable#containeryard
```

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

`yard build` inserts a cache-busting argument before each module fragment by default:

```dockerfile
ARG CACHE_BUST_<MODULE_NAME>=1
```

This lets you bust the build cache for specific modules using podman/docker's `--build-arg` flag, e.g.:

```bash
podman build --build-arg CACHE_BUST_RUST_ESSENTIALS=$(date +%s) -t my-app .
```

Module names come from the `yard.yaml` output declarations (e.g. `rust-essentials:`).

Use `yard build --no-cache-busting` to omit these arguments.

### Embedded Scripts

Declare a named `bash` or `sh` block and reference it at the end of `RUN`:

````markdown
```bash name=install
set -euo pipefail
packages=(curl git)
apt-get update
apt-get install -y "${packages[@]}"
```

```containerfile name=main
RUN {{ script(name="install") }}
```
````

This generates a quoted heredoc with a collision-free delimiter, executed during the image build by the fence's interpreter. The interpreter must be installed in the image. Script bodies are literal by default, preserving shell syntax and Tera-like text.

`template=true` renders the script with declared module arguments using expressions such as `{{ version }}`. Values are rendered once; recursive `script()` calls are unsupported.

`RUN` flags go before the helper, with no other commands before or after it:

```dockerfile
RUN --mount=type=cache,target=/var/cache/apt {{ script(name="install") }}
```

Names are case-sensitive, unique within a module, and available to all its fragments. They may contain ASCII letters, digits, underscores, and hyphens. Use longer or tilde fences for scripts containing backtick fences.

A heredoc-capable builder is required. Outputs default to `# syntax=docker/dockerfile:1` unless explicitly configured. Leading `syntax`, `escape`, and `check` directives are hoisted and deduplicated; conflicting values are errors.

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

## Module Repositories

Repositories of module files. Good resource to see more examples and working configs for other users.

- <https://github.com/mcmah309/containeryard_modules.git> - mcmah309's Module Repository. Rust, Flutter, Bash, etc.

**\*Feel free to create a PR to add your own\***
