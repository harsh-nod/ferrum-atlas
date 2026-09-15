import argparse
import copy
import gzip
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).parents[1] / "release.py"
SPEC = importlib.util.spec_from_file_location("atlas_release", SCRIPT)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        # These are archive fixtures, not executable binaries. The real readelf
        # path is exercised separately below without running its inspected input.
        inspector = patch.object(release, "inspect_elf", return_value={
            "tool": "system readelf", "interpreter": release.ELF_INTERPRETER,
            "needed_libraries": ["libc.so.6"], "maximum_glibc_required": "2.34", "maximum_libgcc_required": None,
        })
        inspector.start()
        self.addCleanup(inspector.stop)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.write("Cargo.toml", '[workspace.package]\nversion="0.1.0"\n')
        self.write("Cargo.lock", 'version=4\n[[package]]\nname="dependency"\nversion="1.0.0"\n')
        self.write("rust-toolchain.toml", '[toolchain]\nchannel="1.97.1"\n')
        self.write("web/package.json", json.dumps({"version": "0.1.0"}))
        self.write("web/src/api/types.ts", "export type Fixture = string;\n")
        self.write("web/package-lock.json", json.dumps({"lockfileVersion": 3, "packages": {"": {"version": "0.1.0"}, "node_modules/viewer-dependency": {"version": "1.0.0", "license": "MIT"}}}))
        self.write("web/node_modules/viewer-dependency/package.json", json.dumps({"name": "viewer-dependency", "version": "1.0.0"}))
        self.write("web/node_modules/viewer-dependency/LICENSE", "Viewer dependency license and copyright\n")
        self.write("scripts/release-schema.json", SCRIPT.with_name("release-schema.json").read_text())
        for document in release.PACKAGED_DOCS:
            self.write(document, "Operation documentation fixture\n")
        for license_name in ("LICENSE-MIT", "LICENSE-APACHE"):
            self.write(license_name, "Project license fixture\n")
        self.write("dependency/LICENSE-MIT", "Cargo dependency license and copyright\n")
        self.metadata = {
            "workspace_root": str(self.repo), "workspace_members": ["workspace"],
            "packages": [
                {"id": "workspace", "name": "atlas", "version": "0.1.0"},
                {"id": "dependency", "name": "dependency", "version": "1.0.0", "license": "MIT", "source": "registry+https://github.com/rust-lang/crates.io-index", "manifest_path": str(self.repo / "dependency/Cargo.toml"), "license_file": None},
            ],
        }
        self.write("metadata.json", json.dumps(self.metadata))
        binary = bytearray(64)
        binary[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<HH", binary, 16, 2, 62)
        self.write("built/atlas", binary)
        self.write("built/web/index.html", '<script src="/assets/main.js"></script>')
        self.write("built/web/assets/main.js", "console.log('fixture');")
        self.write("built/web/assets/main.css", "body { color: black; }")
        self.args = argparse.Namespace(repo=self.repo, version="0.1.0-alpha.1", commit="a" * 40, epoch=1700000000, binary=self.repo / "built/atlas", web_dist=self.repo / "built/web", cargo_metadata=self.repo / "metadata.json", npm_root=self.repo / "web", output=self.root / "output")

    def write(self, path, value):
        destination = self.repo / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(value.encode() if isinstance(value, str) else value)

    def members(self, archive):
        with tarfile.open(archive, "r:gz") as tar:
            return [(copy.copy(member), tar.extractfile(member).read()) for member in tar]

    def altered(self, change, reseal=False):
        original = release.package(self.args)
        members = self.members(original)
        change(members)
        if reseal:
            data = {member.name.partition("/")[2]: payload for member, payload in members}
            manifest = json.loads(data["RELEASE.json"])
            manifest["files"] = {name: {"bytes": len(payload), "sha256": release.sha(payload)} for name, payload in data.items() if name not in ("RELEASE.json", "SHA256SUMS")}
            data["RELEASE.json"] = release.document(manifest)
            data["SHA256SUMS"] = "".join(f"{release.sha(payload)}  {name}\n" for name, payload in sorted(data.items()) if name != "SHA256SUMS").encode()
            members = [(member, data[member.name.partition("/")[2]]) for member, _ in members]
        path = self.root / "altered.tar.gz"
        with tarfile.open(path, "w:gz", format=tarfile.USTAR_FORMAT) as tar:
            for member, data in members:
                member.size = len(data)
                tar.addfile(member, io.BytesIO(data))
        return path

    def test_identical_payloads_are_byte_reproducible_and_checksummed(self):
        first = release.package(self.args)
        os.utime(self.args.binary, (123, 456))
        self.args.output = self.root / "second"
        second = release.package(self.args)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        manifest = release.validate(first)
        self.assertEqual(manifest["qualification"], "unqualified_prerelease")
        self.assertEqual(manifest["source_commit"], "a" * 40)
        self.assertNotIn(str(self.root), json.dumps(manifest))
        self.assertEqual(first.with_suffix(first.suffix + ".sha256").read_text(), f"{release.sha(first.read_bytes())}  {first.name}\n")
        process = subprocess.run([sys.executable, str(SCRIPT), "validate", str(first)], capture_output=True, text=True, timeout=5)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertIn("unqualified local prerelease", process.stdout)

    def test_source_stores_tokens_and_unselected_files_never_enter_package(self):
        self.write(".atlas/token", "SUPER_SECRET_TOKEN")
        self.write(".atlas/source.sqlite", "PRIVATE_SOURCE")
        self.write("src/secret.rs", "PRIVATE_SOURCE")
        self.write("dependency/README.md", "PRIVATE_SOURCE")
        self.write("docs/operations/private-notes.md", "PRIVATE_SOURCE")
        archive = release.package(self.args)
        for _, payload in self.members(archive):
            self.assertNotIn(b"SUPER_SECRET_TOKEN", payload)
            self.assertNotIn(b"PRIVATE_SOURCE", payload)

    def test_metric_guide_is_packaged_but_older_previews_still_validate(self):
        guide = "docs/operations/maintainability.md"
        content = (SCRIPT.parents[1] / guide).read_bytes()
        self.write(guide, content)
        archive = release.package(self.args)
        manifest = release.validate(archive)
        self.assertEqual(manifest["files"][guide], {"bytes": len(content), "sha256": release.sha(content)})
        members = {member.name.partition("/")[2]: payload for member, payload in self.members(archive)}
        self.assertEqual(members[guide], content)
        self.assertIn(b"## Compiler CFG Measure", content)
        self.assertIn(b"## Meaning And Limits", content)
        self.args.output = self.root / "older-preview"
        older = self.altered(lambda members: members.__setitem__(slice(None), [item for item in members if not item[0].name.endswith("/" + guide)]), reseal=True)
        self.assertNotIn(guide, release.validate(older)["files"])
        (self.repo / guide).unlink()
        self.args.output = self.root / "missing-guide"
        with self.assertRaises((ValueError, FileNotFoundError)):
            release.package(self.args)

    def test_packaged_external_references_are_explicit_and_commit_pinned(self):
        root = "https://github.com/harsh-nod/ferrum-atlas/blob/606b519f0812208bb311cc468e9cb539ea5e0f39/"
        references = (
            ("docs/dependencies.md", "docs/milestones/package-verification.md", "online source-checkout package verification reference", "(milestones/package-verification.md)"),
            ("docs/operations/local.md", "benchmarks/README.md", "online source-checkout qualification reference", "(../../benchmarks/README.md)"),
            ("docs/operations/compiler-and-jobs.md", "adapters/rustc/README.md", "online source-checkout producer contract", "(../../adapters/rustc/README.md)"),
        )
        for document, target, label, old in references:
            with self.subTest(document=document):
                content = (SCRIPT.parents[1] / document).read_text()
                self.assertIn(f"[{label}]({root}{target})", content)
                self.assertIn("not bundled with the application", " ".join(content.split()))
                self.assertNotIn(old, content)
                self.write(document, content)
        archive = release.package(self.args)
        members = {member.name.partition("/")[2]: payload for member, payload in self.members(archive)}
        for document, target, label, _ in references:
            self.assertIn(f"[{label}]({root}{target})".encode(), members[document])

    def test_viewer_source_maps_and_unexpected_paths_are_rejected(self):
        for name in ("assets/main.js.map", "token.json", ".atlas/source.sqlite", "assets/nested/private.js"):
            with self.subTest(name=name):
                self.write(f"built/web/{name}", "secret")
                with self.assertRaises(ValueError):
                    release.package(self.args)
                path = self.args.web_dist / name
                path.unlink()
                if path.parent != self.args.web_dist and path.parent.name != "assets":
                    path.parent.rmdir()

    def test_binary_architecture_and_prerelease_identity_are_checked(self):
        for value in ("0.1.0", "0.2.0-alpha.1", "v0.1.0-alpha.1\nsecret", "0.1.0-rc.01"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                release.version(self.repo, value)
        self.assertEqual(release.version(self.repo, "v0.1.0-alpha.1"), "0.1.0-alpha.1")
        self.write("built/atlas", b"#!/bin/sh\necho not ELF\n")
        with self.assertRaisesRegex(ValueError, "ELF"):
            release.package(self.args)
        self.args.commit = "main"
        with self.assertRaisesRegex(ValueError, "Git SHA"):
            release.package(self.args)

    def test_symlink_and_fifo_inputs_fail_without_execution_or_blocking(self):
        saved = self.repo / "built/saved"
        self.args.binary.rename(saved)
        self.args.binary.symlink_to(saved)
        with self.assertRaises(OSError):
            release.package(self.args)
        self.args.binary.unlink()
        os.mkfifo(self.args.binary)
        command = [sys.executable, str(SCRIPT), "package", "--repo", str(self.repo), "--version", self.args.version, "--commit", self.args.commit, "--epoch", str(self.args.epoch), "--binary", str(self.args.binary), "--web-dist", str(self.args.web_dist), "--cargo-metadata", str(self.args.cargo_metadata), "--npm-root", str(self.args.npm_root), "--output", str(self.args.output)]
        process = subprocess.run(command, capture_output=True, text=True, timeout=2)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn("not a regular file", process.stderr)

    def test_missing_licenses_and_stale_resolved_versions_block_packaging(self):
        notice = self.repo / "dependency/LICENSE-MIT"
        notice.unlink()
        with self.assertRaisesRegex(ValueError, "license/notice"):
            release.package(self.args)
        notice.write_text("restored notice")
        self.metadata["packages"][1]["version"] = "2.0.0"
        self.write("metadata.json", json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, "locked package versions"):
            release.package(self.args)
        self.metadata["packages"][1]["version"] = "1.0.0"
        self.write("metadata.json", json.dumps(self.metadata))
        self.write("web/node_modules/viewer-dependency/package.json", '{"version":"2.0.0"}')
        with self.assertRaisesRegex(ValueError, "npm version"):
            release.package(self.args)

    def test_archive_input_rejects_links_fifos_and_directories(self):
        archive = release.package(self.args)
        link = self.root / "link.tar.gz"
        link.symlink_to(archive)
        with self.assertRaises(OSError):
            release.validate(link)
        with self.assertRaises((ValueError, OSError)):
            release.validate(self.root)
        fifo = self.root / "pipe.tar.gz"
        os.mkfifo(fifo)
        process = subprocess.run([sys.executable, str(SCRIPT), "validate", str(fifo)], capture_output=True, text=True, timeout=2)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn("not a regular file", process.stderr)

    def test_empty_notice_text_is_rejected_when_packaged_or_resealed(self):
        self.write("dependency/LICENSE-MIT", " \n\t")
        with self.assertRaisesRegex(ValueError, "empty dependency notice"):
            release.package(self.args)
        self.write("dependency/LICENSE-MIT", "Restored notice")
        def empty(members):
            for index, (member, _) in enumerate(members):
                if member.name.endswith("/dependency-1.0.0/LICENSE-MIT"):
                    members[index] = (member, b"")
        archive = self.altered(empty, reseal=True)
        with self.assertRaisesRegex(ValueError, "empty dependency notice"):
            release.validate(archive)

    def test_required_missing_npm_package_and_license_escape_are_rejected(self):
        self.metadata["packages"][1]["license_file"] = "../../secret"
        self.write("metadata.json", json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, "package-relative"):
            release.package(self.args)
        self.metadata["packages"][1]["license_file"] = None
        self.write("metadata.json", json.dumps(self.metadata))
        (self.repo / "web/node_modules/viewer-dependency").rename(self.repo / "web/removed")
        with self.assertRaisesRegex(ValueError, "not installed"):
            release.package(self.args)

    def test_existing_outputs_are_never_overwritten(self):
        archive = release.package(self.args)
        original = archive.read_bytes()
        with self.assertRaisesRegex(ValueError, "already exist"):
            release.package(self.args)
        self.assertEqual(archive.read_bytes(), original)

    def test_duplicate_archive_member_is_rejected(self):
        archive = self.altered(lambda members: members.append(copy.deepcopy(members[-1])))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            release.validate(archive)

    def test_traversal_and_links_are_rejected_without_extracting(self):
        def traversal(members):
            members[0][0].name = "../../token"
        archive = self.altered(traversal)
        with self.assertRaisesRegex(ValueError, "unsafe archive path"):
            release.validate(archive)
        self.args.output = self.root / "link-package"
        def symlink(members):
            members[0][0].type = tarfile.SYMTYPE
            members[0][0].linkname = "/etc/passwd"
        archive = self.altered(symlink)
        with self.assertRaisesRegex(ValueError, "links"):
            release.validate(archive)

    def test_unexpected_file_rejected_even_with_resealed_checksums(self):
        def extra(members):
            member = copy.copy(members[0][0])
            member.name = member.name.split("/")[0] + "/token.txt"
            members.append((member, b"not permitted"))
        archive = self.altered(extra, reseal=True)
        with self.assertRaisesRegex(ValueError, "unexpected"):
            release.validate(archive)

    def test_tampering_and_missing_required_payload_are_rejected(self):
        def tamper(members):
            for index, (member, _) in enumerate(members):
                if member.name.endswith("/assets/main.js"):
                    members[index] = (member, b"tampered")
        archive = self.altered(tamper)
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            release.validate(archive)
        self.args.output = self.root / "missing-package"
        archive = self.altered(lambda members: members.__setitem__(slice(None), [item for item in members if not item[0].name.endswith("/bin/atlas")]), reseal=True)
        with self.assertRaisesRegex(ValueError, "required release payload"):
            release.validate(archive)

    def test_unsupported_schema_is_rejected_even_when_resealed(self):
        def schema(members):
            for index, (member, data) in enumerate(members):
                if member.name.endswith("/schema-compatibility.json"):
                    value = json.loads(data)
                    value["model_schema_version"] = 99
                    members[index] = (member, release.document(value))
        archive = self.altered(schema, reseal=True)
        with self.assertRaisesRegex(ValueError, "schema compatibility"):
            release.validate(archive)

    def test_file_count_and_byte_budgets_are_checked_before_materializing_data(self):
        archive = release.package(self.args)
        with patch.object(release, "MAX_FILES", 1), self.assertRaisesRegex(ValueError, "file count"):
            release.validate(archive)
        with patch.object(release, "MAX_TOTAL", 16), self.assertRaisesRegex(ValueError, "exceeds budget"):
            release.validate(archive)
        giant = self.root / "giant.tar.gz"
        header = tarfile.TarInfo("ferrum-atlas-0.1.0-alpha.1-x86_64-unknown-linux-gnu/bin/atlas")
        header.size, header.mode = release.MAX_BINARY + 1, 0o755
        with gzip.open(giant, "wb") as stream:
            stream.write(header.tobuf(format=tarfile.USTAR_FORMAT))
        with self.assertRaisesRegex(ValueError, "member exceeds budget"):
            release.validate(giant)

    def test_hidden_extension_headers_are_rejected_before_their_body_parser_runs(self):
        for kind, method in [(tarfile.XHDTYPE, "_proc_pax"), (tarfile.XGLTYPE, "_proc_pax"),
                             (tarfile.GNUTYPE_LONGNAME, "_proc_gnulong"), (tarfile.GNUTYPE_LONGLINK, "_proc_gnulong"),
                             (tarfile.GNUTYPE_SPARSE, "_proc_sparse")]:
            archive = self.root / f"extension-{kind.hex()}.tar.gz"
            header = tarfile.TarInfo("hidden-extension")
            header.type, header.size = kind, release.MAX_TOTAL
            with gzip.open(archive, "wb") as stream:
                stream.write(header.tobuf(format=tarfile.USTAR_FORMAT))
            with self.subTest(kind=kind), patch.object(tarfile.TarInfo, method, side_effect=AssertionError("extension body parser must never run")):
                with self.assertRaisesRegex(ValueError, "extended headers"):
                    release.validate(archive)

    def test_accumulation_budget_is_enforced_before_all_notices_are_loaded(self):
        with self.assertRaisesRegex(ValueError, "budget"):
            release.inventory(self.repo, self.args.cargo_metadata, self.args.npm_root, max_bytes=1)
        payloads = release.Payloads(max_bytes=5)
        payloads["first"] = b"1234"
        with self.assertRaisesRegex(ValueError, "accumulation"):
            payloads["second"] = b"56"
        self.assertEqual(payloads.total, 4)

    def test_missing_dependency_notices_rejected_even_when_resealed(self):
        def missing(members):
            for index, (member, data) in enumerate(members):
                if member.name.endswith("/dependency-inventory.json"):
                    value = json.loads(data)
                    value["packages"][0]["notices"] = []
                    members[index] = (member, release.document(value))
        archive = self.altered(missing, reseal=True)
        with self.assertRaisesRegex(ValueError, "no notice texts"):
            release.validate(archive)

    def test_missing_javascript_and_css_rejected_even_when_resealed(self):
        for extension, reason in ((".js", "JavaScript"), (".css", "stylesheet")):
            self.args.output = self.root / ("without" + extension)
            archive = self.altered(lambda members: members.__setitem__(slice(None), [item for item in members if not item[0].name.endswith(extension)]), reseal=True)
            with self.assertRaisesRegex(ValueError, reason):
                release.validate(archive)

    def supplemental(self):
        data = b"Pinned upstream MIT license fixture\n"
        self.write("scripts/release-notices/dependency/LICENSE", data)
        value = {"format_version": 1, "groups": [{"packages": ["cargo:dependency@1.0.0"], "provenance": "Exact upstream fixture commit", "files": [{"path": "release-notices/dependency/LICENSE", "archive_name": "LICENSE", "url": "https://raw.githubusercontent.com/owner/project/" + "b" * 40 + "/LICENSE", "sha256": release.sha(data)}]}]}
        self.write("scripts/release-notices.json", json.dumps(value))
        (self.repo / "dependency/LICENSE-MIT").unlink()
        return value

    def test_supplemental_notices_are_pinned_checksummed_and_carried_into_inventory(self):
        self.supplemental()
        archive = release.package(self.args)
        release.validate(archive)
        data = {member.name.partition("/")[2]: value for member, value in self.members(archive)}
        inventory = json.loads(data["share/ferrum-atlas/dependency-inventory.json"])
        source = inventory["packages"][0]["supplemental_notice_provenance"][0]
        self.assertIn("b" * 40, source["url"])
        self.assertEqual(source["sha256"], release.sha(data[inventory["packages"][0]["notices"][0]]))

    def test_supplemental_notice_corruption_and_moving_branch_urls_are_rejected(self):
        value = self.supplemental()
        self.write("scripts/release-notices/dependency/LICENSE", "changed")
        with self.assertRaisesRegex(ValueError, "notice checksum"):
            release.package(self.args)
        value["groups"][0]["files"][0]["url"] = "https://raw.githubusercontent.com/owner/project/main/LICENSE"
        self.write("scripts/release-notices.json", json.dumps(value))
        with self.assertRaisesRegex(ValueError, "pin a GitHub source commit"):
            release.package(self.args)

    def test_invalid_declared_identity_runtime_and_abi_are_rejected_when_resealed(self):
        changes = [
            (lambda value: value.update(source_commit="main"), "source commit"),
            (lambda value: value.update(source_date_epoch=True), "timestamps"),
            (lambda value: value.update(source_date_epoch=0), "timestamps"),
            (lambda value: value["runtime_requirements"].update(glibc_minimum="2.17"), "runtime requirements"),
            (lambda value: value["runtime_requirements"].update(procfs=1), "runtime requirements"),
            (lambda value: value["build_inputs"].update(rust_toolchain="nightly"), "tool version"),
            (lambda value: value["build_inputs"].update(cargo_lock_sha256="unknown"), "build-input digest"),
            (lambda value: value["packaging_elf_inspection"].update(interpreter="/tmp/loader"), "ELF interpreter"),
            (lambda value: value["packaging_elf_inspection"].update(needed_libraries=["libunreviewed.so"]), "ELF libraries"),
            (lambda value: value["packaging_elf_inspection"].update(maximum_glibc_required="2.40"), "ELF glibc"),
        ]
        for index, (change, message) in enumerate(changes):
            self.args.output = self.root / f"metadata-{index}"
            def modify(members):
                for offset, (member, data) in enumerate(members):
                    if member.name.endswith("/RELEASE.json"):
                        value = json.loads(data)
                        change(value)
                        members[offset] = (member, release.document(value))
            with self.subTest(index=index), self.assertRaisesRegex(ValueError, message):
                release.validate(self.altered(modify, reseal=True))

    def test_original_v1_without_elf_record_is_structurally_valid_and_missing_docs_are_not(self):
        def legacy(members):
            for index, (member, data) in enumerate(members):
                if member.name.endswith("/RELEASE.json"):
                    value = json.loads(data)
                    del value["packaging_elf_inspection"]
                    members[index] = (member, release.document(value))
        release.validate(self.altered(legacy, reseal=True))
        self.args.output = self.root / "missing-docs"
        with self.assertRaisesRegex(ValueError, "operation documentation"):
            release.validate(self.altered(lambda members: members.__setitem__(slice(None), [item for item in members if not item[0].name.endswith("/docs/operations/install.md")]), reseal=True))
    def test_third_party_notices_are_included_but_license_update_source_is_not(self):
        self.write("dependency/THIRD-PARTY-LICENSE", "Third party licensing")
        self.write("dependency/ThirdPartyNotices.txt", "More copyright notices")
        self.write("dependency/license-update.mjs", "SECRET_BUILD_SCRIPT")
        files = release.license_files(self.repo / "dependency")
        self.assertIn("THIRD-PARTY-LICENSE", files)
        self.assertIn("ThirdPartyNotices.txt", files)
        self.assertNotIn("license-update.mjs", files)


class CheckedInNoticeTests(unittest.TestCase):
    def test_every_checked_in_supplement_has_exact_pinned_source_metadata_and_bytes(self):
        repo = SCRIPT.parent.parent
        overrides = release.supplemental_notices(repo)
        self.assertEqual(len(overrides), 40)
        for group in overrides.values():
            for item in group["files"]:
                self.assertTrue(release.pinned_notice_url(item["url"]))
                data = release.read_regular(repo / "scripts" / item["path"], 1024 * 1024)
                self.assertEqual(release.sha(data), item["sha256"])
        self.assertFalse(release.pinned_notice_url("https://raw.githubusercontent.com/o/r/" + "a" * 40 + "/../main/LICENSE"))


class ELFInspectionTests(unittest.TestCase):
    OUTPUT = """[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]
 0x0000000000000001 (NEEDED)             Shared library: [libgcc_s.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x00a0: Name: GLIBC_2.2.5 Flags: none Version: 2
 0x00b0: Name: GLIBC_2.39 Flags: none Version: 3
 0x00c0: Name: GCC_4.2.0 Flags: none Version: 4
"""

    def test_inspector_rejects_unsupported_loader_dependencies_and_newer_glibc(self):
        self.assertEqual(release.inspect_elf_output(self.OUTPUT)["maximum_glibc_required"], "2.39")
        for output in [self.OUTPUT.replace("2.39", "2.40"), self.OUTPUT.replace("2.39", "PRIVATE"),
                       self.OUTPUT.replace("4.2.0", "7.0.0"),
                       self.OUTPUT.replace("libgcc_s.so.1", "libsurprise.so"), self.OUTPUT.replace(release.ELF_INTERPRETER, "/tmp/loader"),
                       self.OUTPUT + "0x0 (RUNPATH) Library runpath: [/tmp]\n", "not an ELF report"]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                release.inspect_elf_output(output)

    @unittest.skipUnless(shutil.which("readelf") and Path("/bin/true").is_file(), "requires Linux binutils and an installed ELF fixture")
    def test_system_readelf_inspects_bytes_without_executing_them(self):
        result = release.inspect_elf(release.read_regular(Path("/bin/true"), release.MAX_BINARY))
        self.assertEqual(result["interpreter"], release.ELF_INTERPRETER)
        with self.assertRaisesRegex(ValueError, "readelf rejected"):
            release.inspect_elf(b"not an executable")

    def test_inspector_timeout_is_an_explicit_validation_error(self):
        with patch.object(subprocess, "run", side_effect=subprocess.TimeoutExpired("readelf", 15)):
            with self.assertRaisesRegex(ValueError, "time budget"):
                release.inspect_elf(b"fixture")


if __name__ == "__main__":
    unittest.main()
