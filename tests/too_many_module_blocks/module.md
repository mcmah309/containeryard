```yaml name=config
description: "A module with an unknown fragment name"
```
```containerfile name=build
FROM alpine AS builder
```
```containerfile name=main
RUN echo install
```
```containerfile name=finalize
RUN echo finalize
```
```containerfile name=extra
RUN echo extra
```
