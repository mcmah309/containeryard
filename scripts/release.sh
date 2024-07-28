#!/usr/bin/env bash
set -euo pipefail

#version=v0.0.0
#git tag --delete $version
git tag -a $version -m "$version"
git push origin $version