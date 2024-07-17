# Container Yard

Container Yard is a declarative reusable decentralized approach for defining containers. Think Nix flakes meets Containerfiles (aka Dockerfiles).

Container Yard introduces the concept of modules to Containerfiles. 
A module is [Tera](https://keats.github.io/tera/docs/) template for part of a Containerfile representing some specific functionality. e.g. The [rust module](todo) defines rust's installation. Modules can be combined with other modules to create a Containerfile.

A `yard.yaml` file is used to compose modules into Containerfiles.

## `yard.yaml` Example Specification
```yaml
inputs:
  # Modules found on local paths
  paths:
    moduleName1: path/to/module
    moduleName2: path/to/module
  # Modules found in a remote repos
  remotes:
    - url: http://example.com
      ref: v1.0
      paths:
        moduleName3: path/to/module
        moduleName4: path/to/module

outputs:
  # Output Containerfile name
  containerFile1:
    # Name of the module
    moduleName1:
      templateVarName1: value # use `value` for `templateVarName1`
      templateVarName2: # use env variable for template variable `templateVarName2`
      ...
  containerFile2:
...
```

## Templates
Container Yard allows using templates to easily setup projects.

### Initialization
`yard.yaml` can be created with or without templates. You can create your own templates to get your projects up and running fast.

#### Default Template

```bash
yard init .
```

#### Local Template
Initialize a `yard.yaml` file from a local template.

1. At some point save a local template

    Save the current `yard.yaml` file as a template with the current directory's name.
    ```bash
    yard save .
    ```
    Or save the current `yard.yaml` file as a template with the specified name.
    ```bash
    yard save . -t python
    ```

2. At a later point initialize the `yard.yaml` file.

    Create a `yard.yaml` file from the python template.
    ```bash
    yard init . -t python
    ```
#### Remote Template
See [Creating A Template Repository](#creating-a-template-repository)

### List Templates

```bash
yard list -t
```

### Delete Templates

```bash
yard delete -t <NAME>
```

## Creating A Template Repository
Template repositories are used to save pre-configured `yard.yaml` files.
```
yard.yaml # `<USER>.<REPO>`
python.yard.yaml # `<USER>.<REPO>.python`
```
Which are imported with
```bash
yard save --remote <REF> <REPO_URL> <PATH>
```
`<PATH>` is optional.

These then can be used to generate templates locally.
```bash
yard init . -t <USER>.<REPO>.python
```

## Declaring a module

A module is defined by creating two files - `<MODULE_NAME>.Containerfile` and `<MODULE_NAME>.yaml`.

`<MODULE_NAME>.Containerfile` is the Tera template for the Containerfile part.

```Containerfile
COPY {{ app_source }} /app

WORKDIR /app

RUN pip install -r requirements.txt
```

>Note: When using commands such as `COPY` in `<MODULE_NAME>.Containerfile`, `COPY` cannot reference any file above it's current directory.

`<MODULE_NAME>.yaml` is mainly a list of arguments expected by the module
```yaml
name: example_module
description: "This is an example module"
args:
  - app_source
```

## Creating A Module Repository
Module repositories are used to save and load pre-configured modules. A module repository can be any git repo. See [`yard.yaml` Example Specification](#yardyaml-example-specification)

## Why Use Container Yard Over Nix Flakes

Nix flakes guarantees reproducibility at the cost of developer flexibility. Container Yard is decentralized, allowing users to easily use different package managers and upstreams. As such, Container Yard sacrifices some reproducibility guarantees and gains complete developer flexibility.

Container Yard is also built on familiar developer tools - Containerfiles and Tera templates.

## Module Repositories

- <https://github.com/mcmah309/containeryard_repository.git> - The Official Module Repository.

**\*Feel free to create a PR to add your own!\***

