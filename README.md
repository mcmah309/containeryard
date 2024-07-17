# containeryard

Containeryard introduces the concept of modules to Containerfiles (aka Dockerfiles). 
A module is a Tera template for part of a Containerfile, which can be combined with other modules to create a containerfiles.

A yard.yaml file is used to compose modules into Containerfiles

## yard.yaml Example Specification
```yaml
inputs:
    name1: path/to/module
    name2:
        url:
        ref:

outputs:
   containerFile1:
        name1:
            key1: value
            key2: # env variable
        name2.path.to.module:
            ...
    containerFile2:
...
```

## Templates
containeryard allows using templates to easily setup projects.

### Initialization
yard.yaml can be created with or without templates. You can create your own templates to get your projects up and running fast.

#### No Template

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
    Save the current `yard.yaml` file as a template with the specified name.
    ```bash
    yard save . -t python
    ```

2. At a later point initialize the `yard.yaml` file.

    Create a `yard.yaml` file from the python template.
    ```bash
    yard init . -t python
    ```
#### Remote Template
Initialize a `yard.yaml` file from a remote template.

1. At some point add the templates found in a remote repository.

    Save all templates repository.
    ```bash
    yard save <REPO_URL>
    ```
2. Initialize the `yard.yaml` file.
    ```bash
    yard init . -t <USER>.<REPO>.python
    ```

### List Templates

```bash
yard list -t
```

### Delete Templates

```bash
yard delete -t <NAME>
```

## Creating A Template Repository
Template repositories are used to save pre-configured `yard.yaml` files
A valid template repository contains a top level `yard-repo.yaml` file,
which contains the paths to directories containing `yard.yaml` files.
```yaml
- path/to/dir1/
- path/to/dir2/
```
Which may correspond to
```
path/to/dir1/
    yard.yaml # `<USER>.<REPO>.dir`
    python.yaml # `<USER>.<REPO>.python`
path/to/dir2/
    yard.yaml # `<USER>.<REPO>.rust`
```
Which are imported with
```bash
yard save <REPO_URL>
```
These then can be used to generate templates locally.
```bash
yard init . -t <USER>.<REPO>.python
```

## Declaring a module

A module is defined by creating two files - `<MODULE_NAME>.Containerfile` and `<MODULE_NAME>.yaml`.

`<MODULE_NAME>.Containerfile` is the Tera template for the Containerfile part.

todo example

`<MODULE_NAME>.yaml` is mainly a list of arguments expected by the module
```yaml
name: example_module
description: "This is an example module"
args:
    - key1
    - key2
```

## Creating A Module Repository
Module repositories are used to save and load pre-configured modules. A module repository can be any git repo. The individual modules are referenced by the path.
```yaml
inputs:
    name:
        url:
        ref:

outputs:
   containerFile:
        name.path.to.module:
            key1: value
            key2: # env variable
```

## Todo

- Use Tera
- Use yaml to build layers 
- allow specifying paths or git url for templates to import. Git urls require a commit hash.