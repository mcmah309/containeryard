# Container Yard

Container Yard is a declarative, reproducible, and reusable decentralized approach for defining containers. Think Nix flakes meets Containerfiles (aka Dockerfiles).

Container Yard breaks Containerfiles into modules. Modules represent some specific functionality of a container. e.g. The [rust module](https://github.com/mcmah309/containeryard_repository/tree/master/apt/rust/base) defines rust's installation. Modules also support [Tera](https://keats.github.io/tera/docs/) templating.

A `yard.yaml` file is used to compose modules into Containerfiles.
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-schema.json

inputs:
  # Modules found on local paths
  paths:
    finalizer: local_modules/finalizer
  # Modules found in a remote repos
  remotes:
    - url: https://github.com/mcmah309/containeryard_repository
      commit: 992eac4ffc0a65d7e8cd30597d93920901fbd1cd
      paths:
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

The above example is of a `yard.yaml` file composes modules to create containerfiles.

To build the Containfiles defined in a `yard.yaml` file, simply run `yard build .`

## Declaring A Simple Module

A module consists of a [Tera](https://keats.github.io/tera/docs/) template named `Containerfile` and a `yard-module.yaml` file 
that defines configuration options and dependencies of the template.

**Containerfile**
```Containerfile
COPY {{ app_source }} /app

WORKDIR /app

RUN pip install -r requirements.txt
```
**yard-module.yaml**
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/mcmah309/containeryard/master/src/schemas/yard-module-schema.json

description: "This is a modules description"
args:
  required:
    - app_source
  optional:
required_files:
  - app_source
```

For more module examples click [here](https://github.com/mcmah309/containeryard_repository/tree/master).

## Installation
```bash
rustup override set nightly
cargo install containeryard
```
`yard` is the cli tool for Container Yard.

## Why Use Container Yard Over Nix Flakes

Nix flakes guarantees reproducibility at the cost of developer flexibility. Container Yard is decentralized, allowing users to easily use different package managers and upstreams. As such, Container Yard sacrifices some reproducibility guarantees and gains complete developer flexibility.

Container Yard is also extremely simple and built on familiar developer tools - Containerfiles and Tera templates.

## Module Repositories

- <https://github.com/mcmah309/containeryard_repository.git> - mcmah309's Module Repository. Rust, Flutter, Bash, etc.

**\*Feel free to create a PR to add your own!\***

