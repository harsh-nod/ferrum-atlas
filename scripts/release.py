#!/usr/bin/env python3
"""Package explicit, prebuilt local artifacts; never walk a source/store tree."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import tarfile
import tempfile
import tomllib

TARGET = "x86_64-unknown-linux-gnu"
MAX_TOTAL = 512 * 1024 * 1024
MAX_BINARY = 256 * 1024 * 1024
MAX_FILES = 8192
DOCS = (
    "docs/operations/install.md", "docs/operations/local.md",
    "docs/operations/portable-snapshots.md", "docs/operations/retention.md",
    "docs/operations/compiler-and-jobs.md", "docs/dependencies.md",
)
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-(alpha|beta|rc|preview)\.(0|[1-9][0-9]*)")
LICENSE_NAME = re.compile(r"^(licen[sc]es?|copying|copyright|notice|unlicense)([-_.].*)?$", re.I)
ASSET = re.compile(r"assets/[A-Za-z0-9][A-Za-z0-9_.-]*\.(js|css|svg|png|jpg|jpeg|webp|avif|ico|woff|woff2|wasm)")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_regular(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode), f"not a regular file: {path}")
        require(info.st_size <= maximum, f"file exceeds budget: {path}")
        data = stream.read(maximum + 1)
        require(len(data) <= maximum, f"file grew beyond budget: {path}")
        return data


def document(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Payloads(dict):
    def __init__(self, max_bytes=None):
        super().__init__()
        self.limit = MAX_TOTAL if max_bytes is None else max_bytes
        self.total = 0

    @property
    def remaining(self):
        return self.limit - self.total

    def __setitem__(self, name, data):
        require(name not in self, "duplicate payload identity")
        require(len(self) < MAX_FILES + 2 and len(data) <= self.remaining, "payload accumulation exceeds budget")
        self.total += len(data)
        super().__setitem__(name, data)

    def update(self, other):
        for name, data in other.items():
            self[name] = data


def safe_relative(value):
    path = PurePosixPath(value)
    return (isinstance(value, str) and value and len(value) <= 240
            and not path.is_absolute() and "\\" not in value
            and all(part not in ("", ".", "..") for part in value.split("/"))
            and all(32 < ord(char) < 127 for char in value))


def version(repo, supplied):
    value = supplied.removeprefix("v")
    require(len(value) <= 64 and VERSION.fullmatch(value), "version must be MAJOR.MINOR.PATCH-(alpha|beta|rc|preview).N")
    cargo = tomllib.loads(read_regular(repo / "Cargo.toml", 1024 * 1024).decode())
    npm = json.loads(read_regular(repo / "web/package.json", 1024 * 1024))
    base = value.split("-", 1)[0]
    require(base == cargo["workspace"]["package"]["version"] == npm["version"], "release base version must match Cargo and viewer versions")
    return value


def elf(data):
    require(len(data) >= 64 and data[:6] == b"\x7fELF\x02\x01", "binary must be a little-endian ELF64 executable")
    kind, machine = struct.unpack_from("<HH", data, 16)
    require(kind in (2, 3) and machine == 62, "binary must target Linux x86_64")


def license_files(package, explicit=None, max_bytes=None):
    require(package.is_dir() and not package.is_symlink(), f"missing or symlink package directory: {package}")
    candidates = set()
    with os.scandir(package) as entries:
        for count, entry in enumerate(entries, 1):
            require(count <= 1024, "dependency root entry budget exceeded")
            if LICENSE_NAME.fullmatch(entry.name):
                if entry.is_dir(follow_symlinks=False):
                    with os.scandir(entry.path) as children:
                        for child_count, child in enumerate(children, 1):
                            require(child_count <= 128, "license directory budget exceeded")
                            if child.is_file(follow_symlinks=False):
                                candidates.add(Path(entry.name) / child.name)
                else:
                    candidates.add(Path(entry.name))
    if explicit:
        require(safe_relative(explicit), "license_file must be package-relative")
        candidates.add(Path(explicit))
    require(candidates, f"no distributed license/notice text found for {package.name}; review before packaging")
    result = Payloads(max_bytes)
    for relative in sorted(candidates):
        require(safe_relative(relative.as_posix()), "unsupported license filename")
        require(not any(part.is_symlink() for part in (package / relative).parents if part != package.parent), "symlink license parent")
        result[relative.as_posix()] = read_regular(package / relative, min(1024 * 1024, result.remaining))
    return result


def inventory(repo, metadata_path, npm_root, max_bytes=None):
    metadata = json.loads(read_regular(metadata_path, 32 * 1024 * 1024))
    cargo_lock = tomllib.loads(read_regular(repo / "Cargo.lock", 8 * 1024 * 1024).decode())
    cargo_versions = {(item["name"], item["version"]) for item in cargo_lock["package"]}
    require(metadata.get("workspace_root") == str(repo), "Cargo metadata belongs to another checkout")
    lock = json.loads(read_regular(repo / "web/package-lock.json", 8 * 1024 * 1024))
    require(lock.get("lockfileVersion") == 3 and isinstance(lock.get("packages"), dict), "npm lockfile version 3 required")
    require(len(metadata["packages"]) <= 1024 and len(lock["packages"]) <= 2048, "dependency count exceeds budget")
    members = set(metadata["workspace_members"])
    notices, packages = Payloads(max_bytes), []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["id"] in members:
            continue
        require(package.get("license"), f"missing declared Cargo license: {package['name']}")
        require((package["name"], package["version"]) in cargo_versions, "Cargo metadata does not match locked package versions")
        require(isinstance(package.get("source"), str) and package["source"].startswith("registry+"), "release needs reviewed registry-only third-party Cargo inputs")
        label = f"cargo/{package['name']}-{package['version']}"
        require(safe_relative(label), "invalid Cargo package identity")
        files = license_files(Path(package["manifest_path"]).parent, package.get("license_file"), notices.remaining)
        paths = []
        for name, data in files.items():
            destination = f"share/ferrum-atlas/notices/{label}/{name}"
            require(destination not in notices, "duplicate dependency notice identity")
            notices[destination] = data
            paths.append(destination)
        packages.append({"ecosystem": "cargo", "name": package["name"], "version": package["version"], "license": package["license"], "notices": paths})
    require(npm_root.is_dir() and not npm_root.is_symlink(), "installed viewer directory required")
    for name, package in sorted(lock["packages"].items()):
        if not name:
            continue
        require(name.startswith("node_modules/") and safe_relative(name), "invalid npm package path")
        require(package.get("license"), f"missing declared npm license: {name}")
        installed = npm_root / name
        record = {"ecosystem": "npm", "name": name, "version": package["version"], "license": package["license"], "installed": installed.exists(), "optional": bool(package.get("optional")), "notices": []}
        if installed.exists():
            actual = json.loads(read_regular(installed / "package.json", 1024 * 1024))
            require(actual.get("version") == package["version"], f"installed npm version differs from lock: {name}")
            for filename, data in license_files(installed, max_bytes=notices.remaining).items():
                destination = f"share/ferrum-atlas/notices/npm/{name.removeprefix('node_modules/')}/{package['version']}/{filename}"
                require(safe_relative(destination) and destination not in notices, "duplicate or invalid npm notice path")
                notices[destination] = data
                record["notices"].append(destination)
        else:
            require(package.get("optional"), f"required locked npm dependency is not installed: {name}")
        packages.append(record)
    return notices, {
        "format_version": 1, "packages": packages,
        "scope": "Resolved Cargo build graph and locked npm graph, including development dependencies; not a precise linked-code SBOM.",
        "review_status": "Declared license metadata and distributed notice texts; independent compatibility, bundled-native, system/toolchain and source-availability review remains required before public release.",
        "security_status": "This package step does not run an advisory scan or establish absence of vulnerabilities."
    }


def web_files(root, max_bytes=None):
    require(root.is_dir() and not root.is_symlink(), "built viewer directory required")
    result = Payloads(max_bytes)
    for directory, directories, files in os.walk(root, followlinks=False):
        relative_dir = Path(directory).relative_to(root)
        for name in directories:
            path = Path(directory) / name
            require(not path.is_symlink() and (relative_dir / name).as_posix() == "assets", "unexpected viewer directory")
        for name in files:
            relative = (relative_dir / name).as_posix()
            require(relative == "index.html" or ASSET.fullmatch(relative), f"unexpected viewer file: {relative}")
            require(len(result) < 1024, "viewer asset count exceeds budget")
            result[f"share/ferrum-atlas/web/{relative}"] = read_regular(Path(directory) / name, min(32 * 1024 * 1024, result.remaining))
    require("share/ferrum-atlas/web/index.html" in result, "built viewer index.html missing")
    require(any(name.endswith(".js") for name in result), "built viewer JavaScript missing")
    require(any(name.endswith(".css") for name in result), "built viewer stylesheet missing")
    return result


def package(args):
    repo = args.repo.resolve()
    release_version = version(repo, args.version)
    require(re.fullmatch(r"[0-9a-f]{40}", args.commit), "source commit must be a full lowercase Git SHA")
    require(0 <= args.epoch <= 0xFFFFFFFF, "SOURCE_DATE_EPOCH must fit a gzip timestamp")
    binary = read_regular(args.binary, MAX_BINARY)
    elf(binary)
    files = Payloads()
    files["bin/atlas"] = binary
    files.update(web_files(args.web_dist, files.remaining))
    for path in DOCS:
        files[path] = read_regular(repo / path, 1024 * 1024)
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        files[f"share/licenses/ferrum-atlas/{name}"] = read_regular(repo / name, 1024 * 1024)
    schema = json.loads(read_regular(repo / "scripts/release-schema.json", 64 * 1024))
    require(schema["qualification"] == "unqualified_prerelease" and schema["deployment"] == "local_single_node", "release qualification metadata is not conservative")
    files["share/ferrum-atlas/schema-compatibility.json"] = document(schema)
    notices, dependencies = inventory(repo, args.cargo_metadata, args.npm_root, files.remaining)
    files.update(notices)
    files["share/ferrum-atlas/dependency-inventory.json"] = document(dependencies)
    require(len(files) <= MAX_FILES and sum(map(len, files.values())) <= MAX_TOTAL, "release payload exceeds budget")
    require(all(safe_relative(name) for name in files), "unsafe package member")
    toolchain = tomllib.loads(read_regular(repo / "rust-toolchain.toml", 64 * 1024).decode())["toolchain"]["channel"]
    manifest = {
        "format_version": 1, "product": "ferrum-atlas", "version": release_version,
        "source_commit": args.commit, "source_date_epoch": args.epoch, "target": TARGET,
        "qualification": "unqualified_prerelease", "deployment": "local_single_node",
        "runtime_requirements": {"os": "Linux", "architecture": "x86_64", "glibc_minimum": "2.39", "procfs": True},
        "build_inputs": {"rust_toolchain": toolchain, "node": "22.22.1", "npm": "11.16.0", "cargo_lock_sha256": sha(read_regular(repo / "Cargo.lock", 8 * 1024 * 1024)), "npm_lock_sha256": sha(read_regular(repo / "web/package-lock.json", 8 * 1024 * 1024)), "transport_types_sha256": sha(read_regular(repo / "web/src/api/types.ts", 4 * 1024 * 1024))},
        "not_included": ["source workspaces", "index stores", "tokens", "portable source exports", "rustc compiler adapter", "Rust/Node toolchains"],
        "limitations": ["No hosted/distributed support or L/S/O/X qualification claim.", "Checksums establish byte integrity, not producer authenticity or security certification.", "Archive bytes are deterministic for identical payloads and metadata; independent bit-reproducible compiler builds are not claimed."],
        "files": {name: {"sha256": sha(data), "bytes": len(data)} for name, data in sorted(files.items())},
    }
    files["RELEASE.json"] = document(manifest)
    files["SHA256SUMS"] = "".join(f"{sha(data)}  {name}\n" for name, data in sorted(files.items())).encode()
    basename = f"ferrum-atlas-{release_version}-{TARGET}"
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"{basename}.tar.gz"
    sidecar = args.output / f"{basename}.tar.gz.sha256"
    require(not archive.exists() and not sidecar.exists(), "release outputs already exist; nothing was overwritten")
    with tempfile.TemporaryDirectory(prefix=".atlas-package-", dir=args.output) as scratch:
        temporary = Path(scratch) / archive.name
        with temporary.open("wb") as raw:
            with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=args.epoch) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as tar:
                    for name, data in sorted(files.items()):
                        info = tarfile.TarInfo(f"{basename}/{name}")
                        info.size, info.mtime = len(data), args.epoch
                        info.mode = 0o755 if name == "bin/atlas" else 0o644
                        tar.addfile(info, io.BytesIO(data))
            raw.flush()
            os.fsync(raw.fileno())
        validate(temporary)
        with temporary.open("rb") as stream:
            checksum = hashlib.file_digest(stream, "sha256").hexdigest()
        os.link(temporary, archive)
        try:
            with sidecar.open("x", encoding="ascii") as stream:
                stream.write(f"{checksum}  {archive.name}\n")
                stream.flush()
                os.fsync(stream.fileno())
        except BaseException:
            archive.unlink()
            raise
    return archive


def allowed_member(name):
    if name in ("RELEASE.json", "SHA256SUMS", "bin/atlas", "share/ferrum-atlas/schema-compatibility.json", "share/ferrum-atlas/dependency-inventory.json"):
        return True
    if name in DOCS:
        return True
    if name in ("share/licenses/ferrum-atlas/LICENSE-MIT", "share/licenses/ferrum-atlas/LICENSE-APACHE"):
        return True
    if name.startswith("share/ferrum-atlas/web/"):
        relative = name.removeprefix("share/ferrum-atlas/web/")
        return relative == "index.html" or bool(ASSET.fullmatch(relative))
    if name.startswith("share/ferrum-atlas/notices/"):
        return name.split("/")[3] in ("cargo", "npm")
    return False


def validate(archive):
    require(archive.stat().st_size <= MAX_TOTAL, "compressed archive exceeds budget")
    hashes, sizes, documents = {}, {}, {}
    prefix, total = None, 0
    with tarfile.open(archive, "r|gz") as tar:
        for member in tar:
            require(len(hashes) < MAX_FILES + 2, "archive file count exceeds budget")
            require(member.isfile() and not member.pax_headers, "archive links, directories, special files and extended headers are forbidden")
            require(safe_relative(member.name), "unsafe archive path")
            top, separator, name = member.name.partition("/")
            require(separator and (prefix is None or top == prefix), "archive must have one versioned root")
            prefix = top
            require(name not in hashes and allowed_member(name), f"unexpected or duplicate archive member: {name}")
            require(0 <= member.size <= MAX_BINARY, "archive member exceeds budget")
            total += member.size
            require(total <= MAX_TOTAL, "archive uncompressed bytes exceed budget")
            require(member.mode == (0o755 if name == "bin/atlas" else 0o644) and member.uid == 0 and member.gid == 0, "unexpected archive ownership or mode")
            stream = tar.extractfile(member)
            digest = hashlib.sha256()
            collected = bytearray()
            keep = name in ("RELEASE.json", "SHA256SUMS", "share/ferrum-atlas/schema-compatibility.json", "share/ferrum-atlas/dependency-inventory.json")
            require(not keep or member.size <= 8 * 1024 * 1024, "archive metadata exceeds budget")
            first = stream.read(min(64, member.size))
            if name == "bin/atlas":
                elf(first)
            digest.update(first)
            if keep: collected.extend(first)
            for chunk in iter(lambda: stream.read(64 * 1024), b""):
                digest.update(chunk)
                if keep: collected.extend(chunk)
            hashes[name], sizes[name] = digest.hexdigest(), member.size
            if keep: documents[name] = bytes(collected)
    require("RELEASE.json" in documents and "SHA256SUMS" in documents, "archive release manifest/checksums missing")
    manifest = json.loads(documents["RELEASE.json"])
    require(manifest.get("format_version") == 1 and manifest.get("product") == "ferrum-atlas" and manifest.get("target") == TARGET, "unsupported release manifest")
    require(VERSION.fullmatch(manifest.get("version", "")), "archive is not a supported prerelease")
    require(prefix == f"ferrum-atlas-{manifest['version']}-{TARGET}", "archive root/version mismatch")
    require(manifest.get("qualification") == "unqualified_prerelease" and manifest.get("deployment") == "local_single_node", "unsupported release claim")
    expected = manifest["files"]
    require(set(expected) == set(hashes) - {"RELEASE.json", "SHA256SUMS"}, "manifest file set mismatch")
    for name, details in expected.items():
        require(details == {"bytes": sizes[name], "sha256": hashes[name]}, f"payload checksum mismatch: {name}")
    lines = documents["SHA256SUMS"].decode("ascii").splitlines()
    expected_lines = [f"{digest}  {name}" for name, digest in sorted(hashes.items()) if name != "SHA256SUMS"]
    require(lines == expected_lines, "internal checksum list mismatch")
    required = {"bin/atlas", "share/ferrum-atlas/web/index.html", "share/ferrum-atlas/schema-compatibility.json", "share/ferrum-atlas/dependency-inventory.json", "share/licenses/ferrum-atlas/LICENSE-MIT", "share/licenses/ferrum-atlas/LICENSE-APACHE"}
    require(required <= hashes.keys(), "required release payload missing")
    require(any(name.startswith("share/ferrum-atlas/web/") and name.endswith(".js") for name in hashes), "viewer JavaScript missing")
    require(any(name.startswith("share/ferrum-atlas/web/") and name.endswith(".css") for name in hashes), "viewer stylesheet missing")
    schema = json.loads(documents["share/ferrum-atlas/schema-compatibility.json"])
    require(schema.get("metadata_version") == 1 and schema.get("qualification") == "unqualified_prerelease" and schema.get("deployment") == "local_single_node", "unsupported schema metadata")
    supported_schema = json.loads(read_regular(Path(__file__).with_name("release-schema.json"), 64 * 1024))
    require(schema == supported_schema, "archive schema compatibility differs from this validator")
    dependencies = json.loads(documents["share/ferrum-atlas/dependency-inventory.json"])
    require(dependencies.get("format_version") == 1, "unsupported dependency inventory")
    for dependency in dependencies["packages"]:
        require(dependency.get("license"), "dependency license missing")
        require(dependency.get("ecosystem") in ("cargo", "npm"), "unsupported dependency ecosystem")
        if dependency["ecosystem"] == "cargo" or dependency.get("installed") is not False:
            require(dependency["notices"], "installed dependency has no notice texts")
        else:
            require(dependency.get("optional") is True, "only absent optional dependencies may omit notices")
        require(all(name in hashes and name.startswith("share/ferrum-atlas/notices/") for name in dependency["notices"]), "dependency notice missing")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("version")
    check.add_argument("--repo", type=Path, default=Path("."))
    check.add_argument("--version", required=True)
    pack = commands.add_parser("package")
    pack.add_argument("--repo", type=Path, default=Path("."))
    for name in ("binary", "web-dist", "cargo-metadata", "npm-root", "output"):
        pack.add_argument(f"--{name}", type=Path, required=True)
    pack.add_argument("--version", required=True)
    pack.add_argument("--commit", required=True)
    pack.add_argument("--epoch", type=int, required=True)
    verify = commands.add_parser("validate")
    verify.add_argument("archive", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "version": print(version(args.repo, args.version))
        elif args.command == "package": print(package(args))
        else:
            manifest = validate(args.archive)
            print(f"Validated Ferrum Atlas {manifest['version']}: {len(manifest['files'])} files; unqualified local prerelease")
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
        parser.exit(1, f"release validation failed: {error}\n")


if __name__ == "__main__":
    main()
