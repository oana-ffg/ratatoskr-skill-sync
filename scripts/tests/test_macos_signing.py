import copy
from pathlib import Path
import stat
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from macos_signing import APP, accepted, release, run, security, validate_candidate_archive, validate_run


class LocalPublisherBoundaries(unittest.TestCase):
    def test_github_actions_cannot_load_local_publisher_credentials(self):
        with patch("macos_signing.sys.platform", "darwin"), patch.dict("os.environ", {"GITHUB_ACTIONS": "true"}):
            with self.assertRaisesRegex(ValueError, "outside GitHub Actions"):
                release(None)

    def test_credential_commands_do_not_expose_failure_output(self):
        failure = subprocess.CompletedProcess([], 1, stdout=b"private output", stderr=b"private output")
        with patch("macos_signing.subprocess.run", return_value=failure):
            with self.assertRaisesRegex(ValueError, "credential output suppressed") as error:
                run("gopass", "show", "entry", sensitive=True)
            self.assertNotIn("private output", str(error.exception))

    def test_keychain_diagnostics_redact_supplied_password(self):
        failure = subprocess.CompletedProcess([], 1, stdout=b"", stderr=b"SecKeychainItemImport: secret-password rejected")
        with patch("macos_signing.subprocess.run", return_value=failure) as consumer:
            with self.assertRaises(ValueError) as error:
                security("import", "encrypted.p12", "-P", "secret-password")
            self.assertNotIn("secret-password", str(error.exception))
            self.assertNotIn("secret-password", str(consumer.call_args.args))
            self.assertIn(b"secret-password", consumer.call_args.kwargs["input"])

    def test_only_final_apple_acceptance_passes(self):
        accepted({"status": "Accepted"})
        for result in [{}, {"status": "Rejected"}, {"status": "In Progress"}]:
            with self.assertRaisesRegex(ValueError, "did not accept"):
                accepted(result)

    def test_run_must_be_successful_main_dispatch_for_exact_commit(self):
        metadata = {"event": "workflow_dispatch", "headBranch": "main", "headSha": "reviewed",
                    "conclusion": "success", "workflowName": "Build release candidates"}
        validate_run(metadata, "reviewed")
        for field, value in [("event", "pull_request"), ("headBranch", "feature"),
                             ("headSha", "other"), ("conclusion", "failure"),
                             ("workflowName", "CI")]:
            changed = copy.copy(metadata)
            changed[field] = value
            with self.assertRaisesRegex(ValueError, "successful main-branch"):
                validate_run(changed, "reviewed")

    def test_candidate_rejects_path_traversal_and_external_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate.zip"
            for name in ["/outside", f"{APP}/../outside", "outside/file", "outside\\file"]:
                with zipfile.ZipFile(path, "w") as archive:
                    archive.writestr(name, "untrusted candidate")
                with self.assertRaises(ValueError):
                    validate_candidate_archive(path)
            for target in ["/outside", "../../outside"]:
                with zipfile.ZipFile(path, "w") as archive:
                    link = zipfile.ZipInfo(f"{APP}/escape")
                    link.external_attr = (stat.S_IFLNK | 0o777) << 16
                    archive.writestr(link, target)
                with self.assertRaisesRegex(ValueError, "symlink escapes"):
                    validate_candidate_archive(path)

    def test_candidate_preserves_internal_framework_links(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate.zip"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr(f"{APP}/Contents/Info.plist", "public candidate")
                link = zipfile.ZipInfo(f"{APP}/Contents/Frameworks/Example.framework/Versions/Current")
                link.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(link, "A")
            validate_candidate_archive(path)
