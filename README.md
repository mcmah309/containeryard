# ContainerYard

ContainerYard is a declarative, reproducible, and reusable decentralized approach for defining containers. Think Nix flakes meets Containerfiles (aka Dockerfiles).

ContainerYard breaks Containerfiles into modules. Modules represent some specific functionality of a container. e.g. The [rust module](https://github.com/mcmah309/yard_module_repository/tree/3c81a4a383f4446437df364ef0a6ba17bc88c479/dependent/apt/rust) defines rust's installation. Modules also support [Tera](https://keats.github.io/tera/docs/#templates) templating.

A `yard.yaml` file is used to compose modules into Containerfiles.
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-schema.json

inputs:
  # Modules found on local paths
  modules:
    finalizer: local_modules/finalizer
  # Modules found in a remote repos
  remotes:
    - url: https://github.com/mcmah309/yard_module_repository
      commit: 992eac4ffc0a65d7e8cd30597d93920901fbd1cd
      modules:
        base: bases/ubuntu/base
        git_config: independent/git_config
        bash_flavor: apt/bash_interactive/flavors/mcmah309

outputs:
  # Output Containerfile created from modules
  Containerfile:
    # Module "base" from inputs
    - base:
        version: "24.04"
    # Inline modules
    - RUN apt install git
    - git_config:
        # Template
        user_name: test_user
        email: test_user@email.com
    - bash_flavor:
    - finalizer:
```

To compose the modules defined in `yard.yaml` into Containerfiles, simply run `yard build .`.
Which in the above case, will output a single Containerfile to your current directory.

## Declaring A Simple Module

A module consists of a [Tera](https://keats.github.io/tera/docs/#templates) template named `Containerfile` and a `yard-module.yaml` file 
that defines configuration options and dependencies of the template.

**Containerfile**
```Containerfile
FROM alpine:{{ version | default (value="latest") }}

RUN apk update \
    && apk upgrade \
    && apk add --no-cache ca-certificates \
    && update-ca-certificates
```
**yard-module.yaml**
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "This is a modules description"
args:
  required:
  optional:
    - version
required_files:
```

For more module examples click [here](https://github.com/mcmah309/yard_module_repository/tree/master).

## Installation

Note: `yard` is the cli tool for ContainerYard.

### Debian - Ubuntu, Linux Mint, Pop!_OS, etc.

```bash
RELEASE_VER=<INSERT_CURRENT_VERSION> # e.g. RELEASE_VER='v0.2.2'
deb_file="containeryard_$(echo $RELEASE_VER | sed 's/^v//')-1_amd64.deb"
curl -LO https://github.com/mcmah309/containeryard/releases/download/$RELEASE_VER/$deb_file
sudo dpkg -i "$deb_file"
```

### Cargo

```bash
cargo install containeryard
```

## Why Use ContainerYard?

Developers constantly rewrite the same Containerfile/Dockerfile configs. Besides taking away developer time, 
these configs become hard to maintain/upgrade and adding new features feels like starting from scratch again.
The goal of ContainerYard is to foster a library ecosystem of composable Containerfile modules. Users
can then import these various modules with little to no configuration. Want Rust? Just add it to your `yard.yaml` file.
Want Flutter? Do the same. With ContainerYard you should never have to define certain Containerfile configs again. But
if you do want to do something custom, ContainerYard does not get in your way, everything is Containerfile based 
and the output is a pure Containerfile. No need to learn a complex tool, no need to re-invent the wheel, Containerfiles 
and Tera templates are powerful enough. Just let ContainerYard be the glue.

## Why Use Container Yard Over Nix Flakes

Nix flakes guarantees reproducibility at the cost of developer flexibility. Container Yard is decentralized, allowing users to easily use different package managers and upstreams. As such, Container Yard sacrifices some reproducibility guarantees and gains complete developer flexibility.

Container Yard is also extremely simple and built on familiar developer tools - Containerfiles and Tera templates.

## Contributing

Feel free to open an issue with any suggestions/ideas/bugs you may have and/or create PR's.

ContainerYard builds and uses its own dev container :D see [here](https://github.com/mcmah309/containeryard/tree/master/.devcontainer).
Open the project in vscode, click the "open in container" button and you are ready to go! Otherwise just use the provided Containerfile or your own local setup.


## Module Repositories

- <https://github.com/mcmah309/yard_module_repository.git> - mcmah309's Module Repository. Rust, Flutter, Bash, etc.

**\*Feel free to create a PR to add your own!\***

