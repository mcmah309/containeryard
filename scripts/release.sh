#version=v0.0.0
#git tag --delete $version
git tag -a $version -m "$version"
git push origin $version