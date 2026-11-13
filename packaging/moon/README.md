# `.moon` bundles https://moonpkg.github.io/cli / https://github.com/moonpkg/cli

A `.moon` bundle is one file that carries the whole app: the binary, its
manifest, its menu entry and its icon. A user unpacks nothing and adds no
repository:

```sh
moon install quester-1.2.1-linux-x86_64.moon
```

It is a plain `tar.gz`, so `tar xzf quester.moon` still works and the file can be
inspected, diffed or checksummed like any other archive.

## Building one

From the repository root:

```sh
bash packaging/moon/build-moon-bundle.sh
bash packaging/moon/build-moon-bundle.sh --from dist/quester-v1.2.1-linux-x86_64.tar.gz
```

If the `moon` CLI is not installed, the script falls back to plain `tar`; the
resulting file is still a valid `.moon` bundle.

## What ends up inside

```
quester-1.2.1-linux-x86_64.moon   # (tar.gz)
├── quester.manifest              # moon's own key=value manifest, paths relative
├── app/
│   ├── bin/quester               # the binary
│   └── share/
│       ├── doc/quester/README.md
│       └── licenses/quester/LICENSE
├── desktop/quester.desktop       # the menu entry
└── icon/quester.png              # the icon
```

The manifest is the same format moon writes for every install, with the paths
the bundle can have:

```
dir=app
main=app/bin/quester
version=1.2.1
desktop=desktop/quester.desktop
link=quester
to=app/bin/quester
cmd=quester
```
