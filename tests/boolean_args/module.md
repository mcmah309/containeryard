```yaml
args:
  required:
    - enabled
    - disabled
    - message
    - count
    - negative
    - fraction
```

```Containerfile
RUN echo {{ enabled }} {{ disabled }} {{ message }}
RUN echo {{ count + 1 }} {{ negative }} {{ fraction + 0.5 }}
{% if enabled %}RUN echo enabled{% else %}RUN echo unexpected-enabled{% endif %}
{% if disabled %}RUN echo unexpected-disabled{% else %}RUN echo disabled{% endif %}
```
