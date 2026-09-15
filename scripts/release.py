#!/usr/bin/env python3
"""Package explicit, prebuilt local artifacts; never walk a source/store tree."""
import argparse
from contextlib import contextmanager
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import resource
import stat
import struct
import subprocess
import tarfile
import tempfile
import tomllib
from urllib.parse import urlsplit

TARGET = "x86_64-unknown-linux-gnu"
RUNTIME_REQUIREMENTS = {"os": "Linux", "architecture": "x86_64", "glibc_minimum": "2.39", "procfs": True}
ELF_INTERPRETER = "/lib64/ld-linux-x86-64.so.2"
ELF_LIBRARIES = {"libgcc_s.so.1", "libm.so.6", "libc.so.6", "ld-linux-x86-64.so.2"}
MAX_TOTAL = 512 * 1024 * 1024
MAX_BINARY = 256 * 1024 * 1024
MAX_FILES = 8192
DOCS = (
    "docs/operations/install.md", "docs/operations/local.md",
    "docs/operations/portable-snapshots.md", "docs/operations/retention.md",
    "docs/operations/compiler-and-jobs.md", "docs/dependencies.md",
    "docs/operations/state-machines.md",
)
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-(alpha|beta|rc|preview)\.(0|[1-9][0-9]*)")
LICENSE_NAME = re.compile(r"^(third[-_ ]?party[-_ ]?)?(licen[sc]es?|copying|copyright|notices?(text)?|unlicense)([-_.].*)?$", re.I)
NOTICE_SUFFIXES = {"", ".txt", ".md", ".rst", ".html", ".apache", ".mit", ".bsd", ".lesser", ".gpl", ".lgpl", ".0"}
PINNED_NOTICE_URL = re.compile(r"https://raw\.githubusercontent\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/[0-9a-f]{40}/[A-Za-z0-9_./-]+")
ASSET = re.compile(r"assets/[A-Za-z0-9][A-Za-z0-9_.-]*\.(js|css|svg|png|jpg|jpeg|webp|avif|ico|woff|woff2|wasm)")


def require(condition, message):
    if not condition:
        raise ValueError(message)


@contextmanager
def regular_stream(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode), f"not a regular file: {path}")
        require(info.st_size <= maximum, f"file exceeds budget: {path}")
        yield stream


def read_regular(path, maximum):
    with regular_stream(path, maximum) as stream:
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


class MissingNotice(ValueError):
    pass


def safe_relative(value):
    path = PurePosixPath(value)
    return (isinstance(value, str) and value and len(value) <= 240
            and not path.is_absolute() and "\\" not in value
            and all(part not in ("", ".", "..") for part in value.split("/"))
            and all(32 < ord(char) < 127 for char in value))


def pinned_notice_url(value):
    return (isinstance(value, str) and len(value) <= 2048
            and PINNED_NOTICE_URL.fullmatch(value)
            and safe_relative("/".join(urlsplit(value).path.split("/")[4:])))


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


def inspect_elf_output(output):
    interpreters = re.findall(r"\[Requesting program interpreter: ([^\]]+)\]", output)
    require(interpreters == [ELF_INTERPRETER], "unsupported ELF interpreter or non-executable shared object")
    libraries = sorted(set(re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", output)))
    require(libraries and set(libraries) <= ELF_LIBRARIES, "ELF needs an undeclared shared library")
    require(not re.search(r"\((RPATH|RUNPATH)\)", output), "ELF runtime search paths are not supported")
    versions = re.findall(r"Name:\s+(GLIBC_\S+)", output)
    require(versions and all(re.fullmatch(r"GLIBC_[0-9]+(?:\.[0-9]+){1,2}", value) for value in versions), "ELF GLIBC version requirements are missing or unsupported")
    maximum = max((tuple(map(int, value.removeprefix("GLIBC_").split("."))) for value in versions))
    require(maximum <= (2, 39), "ELF requires a newer glibc than the declared minimum")
    gcc_versions = re.findall(r"Name:\s+(GCC_\S+)", output)
    require(all(re.fullmatch(r"GCC_[0-9]+(?:\.[0-9]+){1,2}", value) for value in gcc_versions), "unsupported ELF libgcc version requirement")
    gcc_maximum = max((tuple(map(int, value.removeprefix("GCC_").split("."))) for value in gcc_versions), default=None)
    require("libgcc_s.so.1" not in libraries or gcc_maximum is not None, "ELF libgcc symbol versions missing")
    require(gcc_maximum is None or gcc_maximum <= (4, 2, 0), "ELF requires a newer libgcc than the declared runtime")
    return {"tool": "system readelf", "interpreter": ELF_INTERPRETER, "needed_libraries": libraries,
            "maximum_glibc_required": ".".join(map(str, maximum)),
            "maximum_libgcc_required": ".".join(map(str, gcc_maximum)) if gcc_maximum is not None else None}


def limit_elf_inspector():
    # Inspect data, never execute the payload; bound even malformed ELF diagnostic output.
    resource.setrlimit(resource.RLIMIT_AS, (512 * 1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_FSIZE, (4 * 1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_CPU, (10, 10))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def inspect_elf(data):
    with tempfile.TemporaryFile() as binary, tempfile.TemporaryFile() as output:
        binary.write(data)
        binary.flush()
        try:
            completed = subprocess.run(
                ["readelf", "--wide", "--program-headers", "--dynamic", "--version-info", f"/proc/self/fd/{binary.fileno()}"],
                pass_fds=(binary.fileno(),), stdout=output, stderr=subprocess.STDOUT,
                env={**os.environ, "LC_ALL": "C"}, timeout=15, preexec_fn=limit_elf_inspector,
            )
        except subprocess.TimeoutExpired as error:
            raise ValueError("ELF inspection exceeded its time budget") from error
        require(completed.returncode == 0, "system readelf rejected the ELF payload or exceeded its budget")
        output.seek(0)
        text = output.read(4 * 1024 * 1024 + 1)
        require(len(text) <= 4 * 1024 * 1024, "ELF inspection output exceeds budget")
        return inspect_elf_output(text.decode("utf-8"))


def license_files(package, explicit=None, max_bytes=None):
    require(package.is_dir() and not package.is_symlink(), f"missing or symlink package directory: {package}")
    candidates = set()
    with os.scandir(package) as entries:
        for count, entry in enumerate(entries, 1):
            require(count <= 1024, "dependency root entry budget exceeded")
            if LICENSE_NAME.fullmatch(entry.name) and Path(entry.name).suffix.lower() in NOTICE_SUFFIXES:
                if entry.is_dir(follow_symlinks=False):
                    with os.scandir(entry.path) as children:
                        for child_count, child in enumerate(children, 1):
                            require(child_count <= 128, "license directory budget exceeded")
                            if child.is_file(follow_symlinks=False) and Path(child.name).suffix.lower() in NOTICE_SUFFIXES:
                                candidates.add(Path(entry.name) / child.name)
                else:
                    candidates.add(Path(entry.name))
    if explicit:
        require(safe_relative(explicit), "license_file must be package-relative")
        candidates.add(Path(explicit))
    if not candidates:
        raise MissingNotice(f"no distributed license/notice text found for {package.name}; review before packaging")
    result = Payloads(max_bytes)
    for relative in sorted(candidates):
        require(safe_relative(relative.as_posix()), "unsupported license filename")
        require(not any(part.is_symlink() for part in (package / relative).parents if part != package.parent), "symlink license parent")
        data = read_regular(package / relative, min(1024 * 1024, result.remaining))
        require(data.strip(), "empty dependency notice text")
        result[relative.as_posix()] = data
    return result


def supplemental_notices(repo):
    path = repo / "scripts/release-notices.json"
    if not path.exists():
        return {}
    value = json.loads(read_regular(path, 1024 * 1024))
    require(value.get("format_version") == 1 and len(value["groups"]) <= 64, "unsupported supplemental notice manifest")
    result = {}
    for group in value["groups"]:
        require(1 <= len(group["files"]) <= 32 and len(group["packages"]) <= 1024, "supplemental notice count exceeds budget")
        require(isinstance(group.get("provenance"), str) and len(group["provenance"]) <= 2048, "supplemental provenance exceeds budget")
        for item in group["files"]:
            require(pinned_notice_url(item["url"]), "supplemental notice URL must pin a GitHub source commit")
            require(safe_relative(item["path"]) and item["path"].startswith("release-notices/") and safe_relative(item["archive_name"]), "invalid supplemental notice path")
            require(re.fullmatch(r"[0-9a-f]{64}", item["sha256"]), "invalid supplemental notice digest")
        for name in group["packages"]:
            require(name not in result, "duplicate supplemental package identity")
            result[name] = group
    return result


def dependency_notices(repo, package, identity, overrides, remaining, explicit=None):
    try:
        return license_files(package, explicit, remaining), []
    except MissingNotice:
        group = overrides.get(identity)
        if group is None:
            raise
    files, provenance = Payloads(remaining), []
    for item in group["files"]:
        path = repo / "scripts" / item["path"]
        require(not any(parent.is_symlink() for parent in path.parents), "symlink supplemental notice parent")
        data = read_regular(path, min(1024 * 1024, files.remaining))
        require(data.strip(), "empty supplemental notice text")
        require(sha(data) == item["sha256"], "supplemental notice checksum mismatch")
        files[item["archive_name"]] = data
        provenance.append({"url": item["url"], "sha256": item["sha256"], "notice_name": item["archive_name"], "mapping_basis": group["provenance"]})
    return files, provenance


def inventory(repo, metadata_path, npm_root, max_bytes=None):
    metadata = json.loads(read_regular(metadata_path, 32 * 1024 * 1024))
    cargo_lock = tomllib.loads(read_regular(repo / "Cargo.lock", 8 * 1024 * 1024).decode())
    cargo_versions = {(item["name"], item["version"]) for item in cargo_lock["package"]}
    require(metadata.get("workspace_root") == str(repo), "Cargo metadata belongs to another checkout")
    lock = json.loads(read_regular(repo / "web/package-lock.json", 8 * 1024 * 1024))
    require(lock.get("lockfileVersion") == 3 and isinstance(lock.get("packages"), dict), "npm lockfile version 3 required")
    require(len(metadata["packages"]) <= 1024 and len(lock["packages"]) <= 2048, "dependency count exceeds budget")
    members = set(metadata["workspace_members"])
    overrides = supplemental_notices(repo)
    notices, packages = Payloads(max_bytes), []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["id"] in members:
            continue
        require(package.get("license"), f"missing declared Cargo license: {package['name']}")
        require((package["name"], package["version"]) in cargo_versions, "Cargo metadata does not match locked package versions")
        require(isinstance(package.get("source"), str) and package["source"].startswith("registry+"), "release needs reviewed registry-only third-party Cargo inputs")
        label = f"cargo/{package['name']}-{package['version']}"
        require(safe_relative(label), "invalid Cargo package identity")
        files, provenance = dependency_notices(repo, Path(package["manifest_path"]).parent, f"cargo:{package['name']}@{package['version']}", overrides, notices.remaining, package.get("license_file"))
        paths = []
        for name, data in files.items():
            destination = f"share/ferrum-atlas/notices/{label}/{name}"
            require(destination not in notices, "duplicate dependency notice identity")
            notices[destination] = data
            paths.append(destination)
        packages.append({"ecosystem": "cargo", "name": package["name"], "version": package["version"], "license": package["license"], "notices": paths, "supplemental_notice_provenance": provenance})
    require(npm_root.is_dir() and not npm_root.is_symlink(), "installed viewer directory required")
    for name, package in sorted(lock["packages"].items()):
        if not name:
            continue
        require(name.startswith("node_modules/") and safe_relative(name), "invalid npm package path")
        require(package.get("license"), f"missing declared npm license: {name}")
        installed = npm_root / name
        record = {"ecosystem": "npm", "name": name, "version": package["version"], "license": package["license"], "installed": installed.exists(), "optional": bool(package.get("optional")), "notices": [], "supplemental_notice_provenance": []}
        if installed.exists():
            actual = json.loads(read_regular(installed / "package.json", 1024 * 1024))
            require(actual.get("version") == package["version"], f"installed npm version differs from lock: {name}")
            files, provenance = dependency_notices(repo, installed, f"npm:{name}@{package['version']}", overrides, notices.remaining)
            record["supplemental_notice_provenance"] = provenance
            for filename, data in files.items():
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
    elf_requirements = inspect_elf(binary)
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
        "runtime_requirements": RUNTIME_REQUIREMENTS,
        "packaging_elf_inspection": elf_requirements,
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


class ReleaseTarInfo(tarfile.TarInfo):
    def _proc_member(self, archive):
        # Reject extension/sparse bodies before tarfile allocates or expands them.
        # A check on yielded members is too late for hidden PAX/GNU headers.
        require(self.type in (tarfile.REGTYPE, tarfile.AREGTYPE), "archive links, directories, special files and extended headers are forbidden")
        require(0 <= self.size <= MAX_BINARY, "archive member exceeds budget")
        return super()._proc_member(archive)


def validate(archive):
    hashes, sizes, documents = {}, {}, {}
    prefix, total, mtimes = None, 0, set()
    with regular_stream(archive, MAX_TOTAL) as source, tarfile.open(fileobj=source, mode="r|gz", tarinfo=ReleaseTarInfo) as tar:
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
            mtimes.add(member.mtime)
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
    require(isinstance(manifest.get("source_commit"), str) and re.fullmatch(r"[0-9a-f]{40}", manifest["source_commit"]), "invalid declared source commit")
    epoch = manifest.get("source_date_epoch")
    require(type(epoch) is int and 0 <= epoch <= 0xFFFFFFFF and mtimes == {epoch}, "archive timestamps differ from declared source epoch")
    require(manifest.get("runtime_requirements") == RUNTIME_REQUIREMENTS and type(manifest["runtime_requirements"].get("procfs")) is bool, "unsupported declared runtime requirements")
    inputs = manifest.get("build_inputs")
    require(isinstance(inputs, dict) and set(inputs) == {"rust_toolchain", "node", "npm", "cargo_lock_sha256", "npm_lock_sha256", "transport_types_sha256"}, "invalid declared build inputs")
    for key in ("rust_toolchain", "node", "npm"):
        require(isinstance(inputs[key], str) and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", inputs[key]), "invalid declared tool version")
    for key in ("cargo_lock_sha256", "npm_lock_sha256", "transport_types_sha256"):
        require(isinstance(inputs[key], str) and re.fullmatch(r"[0-9a-f]{64}", inputs[key]), "invalid declared build-input digest")
    # Older v1 packages omit this record. For either form, archive validation is
    # structural: a declaration is not proof of ABI, build origin or authenticity.
    if "packaging_elf_inspection" in manifest:
        inspection = manifest["packaging_elf_inspection"]
        require(isinstance(inspection, dict) and set(inspection) == {"tool", "interpreter", "needed_libraries", "maximum_glibc_required", "maximum_libgcc_required"}, "invalid declared ELF inspection")
        require(inspection["tool"] == "system readelf" and inspection["interpreter"] == ELF_INTERPRETER, "unsupported declared ELF interpreter")
        libraries = inspection["needed_libraries"]
        require(isinstance(libraries, list) and libraries and all(isinstance(value, str) for value in libraries) and libraries == sorted(set(libraries)) and set(libraries) <= ELF_LIBRARIES, "unsupported declared ELF libraries")
        minimum = inspection["maximum_glibc_required"]
        require(isinstance(minimum, str) and re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", minimum) and tuple(map(int, minimum.split("."))) <= (2, 39), "unsupported declared ELF glibc requirement")
        gcc = inspection["maximum_libgcc_required"]
        require((gcc is None and "libgcc_s.so.1" not in libraries) or (isinstance(gcc, str) and re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", gcc) and tuple(map(int, gcc.split("."))) <= (4, 2, 0)), "unsupported declared ELF libgcc requirement")
    expected = manifest["files"]
    require(set(expected) == set(hashes) - {"RELEASE.json", "SHA256SUMS"}, "manifest file set mismatch")
    for name, details in expected.items():
        require(details == {"bytes": sizes[name], "sha256": hashes[name]}, f"payload checksum mismatch: {name}")
    lines = documents["SHA256SUMS"].decode("ascii").splitlines()
    expected_lines = [f"{digest}  {name}" for name, digest in sorted(hashes.items()) if name != "SHA256SUMS"]
    require(lines == expected_lines, "internal checksum list mismatch")
    required = {"bin/atlas", "share/ferrum-atlas/web/index.html", "share/ferrum-atlas/schema-compatibility.json", "share/ferrum-atlas/dependency-inventory.json", "share/licenses/ferrum-atlas/LICENSE-MIT", "share/licenses/ferrum-atlas/LICENSE-APACHE"}
    require(required <= hashes.keys(), "required release payload missing")
    require(set(DOCS) <= hashes.keys(), "required operation documentation missing")
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
        require(all(sizes[name] > 0 for name in dependency["notices"]), "empty dependency notice text")
        for source in dependency.get("supplemental_notice_provenance", []):
            require(pinned_notice_url(source["url"]), "supplemental notice URL is not pinned")
            matched = [name for name in dependency["notices"] if name.endswith("/" + source["notice_name"])]
            require(len(matched) == 1 and hashes[matched[0]] == source["sha256"], "supplemental notice provenance digest mismatch")
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
            print(f"Validated Ferrum Atlas {manifest['version']}: {len(manifest['files'])} files; unqualified local prerelease; structural validation only, ABI and origin declarations unverified")
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
        parser.exit(1, f"release validation failed: {error}\n")


if __name__ == "__main__":
    main()
