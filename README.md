# pxvirt-isoinfo

Detect the operating system on an installation image without mounting it.
ISO 9660 (Joliet, Rock Ridge, multi-extent files) and UDF are read directly, the
editions of Windows images are read from the XML data of `sources/install.wim`
or `install.esd`.

```
pxvirt-isoinfo [--rules FILE] [--pretty] IMAGE
pxvirt-isoinfo --ls PATH IMAGE
pxvirt-isoinfo --dump-rules
```

## Output

```json
{
  "type": "windows",
  "filesystem": "udf",
  "label": "SSS_X64FRE_EN-US_DV9",
  "id": "windows",
  "name": "Windows Server 2022",
  "version": "10.0.20348.587",
  "arch": "x86_64",
  "installer": "windows",
  "ostype": "win11",
  "images": [
    { "index": 1, "name": "Windows Server 2022 Standard", "edition": "ServerStandard",
      "installation_type": "Server Core", "arch": "x86_64", "version": "10.0.20348.587" }
  ]
}
```

`type` is `windows`, `linux` or `unknown`. `installer` is the autoinstall type
(`windows`, `kickstart`, `ubuntu`), it is missing if unattended installation is
not supported. `ostype` is the PXVirt guest OS type. Optional fields are left
out if they are unknown.

## Rules

The rules are read from `/usr/share/pxvirt-isoinfo/rules.json` (a copy is
built into the binary). Local rules in `/etc/pxvirt-isoinfo/rules.d/*.json`
are checked first, they may contain any of `arch_aliases`, `windows.images`,
`windows.versions` and `linux`.

Linux rules are checked in order, the first matching rule wins:

```json
{
    "id": "rocky",
    "name": "Rocky Linux",
    "installer": "kickstart",
    "ostype": "l26",
    "match": {
        "label": "^Rocky",
        "files_all": [".treeinfo"],
        "files_any": ["BaseOS", "AppStream"],
        "content": [{ "file": ".treeinfo", "regex": "(?m)^family\\s*=\\s*Rocky Linux" }]
    },
    "info": [
        { "file": ".treeinfo", "regex": "(?m)^version\\s*=\\s*(?P<version>\\S+)" },
        { "file": ".treeinfo", "regex": "(?m)^arch\\s*=\\s*(?P<arch>\\S+)" }
    ]
}
```

All given `match` conditions must be fulfilled. Paths are case insensitive.
The named groups `name`, `version` and `arch` of the `info` expressions are
reported, the first match of each group wins. `@label` as file matches the
volume label. Architectures are normalized with `arch_aliases`.

Windows `versions` are matched against the build number and installation type
of the first image, the first match sets `name` and `ostype`.

## Building

```
cargo build --release
cargo test               # needs genisoimage
cargo deb
cargo build --release && cargo generate-rpm
```

The CI builds static binaries with `--target x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl`, so the packages work on any distribution release.
