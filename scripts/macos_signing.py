"""Local Apple signing; private inputs go from gopass to consumers in memory."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import plistlib
import posixpath
import re
import secrets
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
APP = "Ratatoskr Skill Sync.app"
BUNDLE_ID = "com.firefungames.ratatoskr"
REPOSITORY = "oana-ffg/ratatoskr-skill-sync"


def run(*args, data=None, sensitive=False, pass_fds=(), env=None):
    result = subprocess.run([str(a) for a in args], input=data, capture_output=True,
                            pass_fds=pass_fds, env=env, cwd=ROOT)
    if result.returncode:
        detail = "" if sensitive else result.stderr.decode(errors="replace").strip()
        raise ValueError(f"{Path(args[0]).name} failed (exit {result.returncode})" +
                         (f": {detail}" if detail else "; credential output suppressed"))
    return result.stdout


def gopass(entry):
    return run("gopass", "show", entry, sensitive=True)


@contextmanager
def input_pipe(data):
    read_fd, write_fd = os.pipe()
    try:
        if len(data) > 4096:
            raise ValueError("Unexpectedly large credential input")
        os.write(write_fd, data)
        os.close(write_fd)
        write_fd = -1
        yield read_fd
    finally:
        os.close(read_fd)
        if write_fd >= 0:
            os.close(write_fd)


def security(*args):
    # Interactive mode keeps password-bearing commands on stdin, out of argv.
    result = subprocess.run(["/usr/bin/security", "-i"],
                            input=(shlex.join(map(str, args)) + "\n").encode(), capture_output=True)
    if result.returncode:
        diagnostic = result.stderr.decode(errors="replace")
        for argument in args[1:]:
            if str(argument) and not str(argument).startswith("-"):
                diagnostic = diagnostic.replace(str(argument), "[redacted]")
        error = re.search(r"(?:Sec[A-Za-z]+|CSSM[A-Z_]+): [^\r\n]+", diagnostic)
        detail = ": " + error.group() if error else "; credential output suppressed"
        raise ValueError(f"security {args[0]} failed (exit {result.returncode}){detail}")
    return result.stdout


def public_key(data, *args):
    return run("openssl", "pkey", *args, "-pubout", "-outform", "DER", data=data, sensitive=True)


@contextmanager
def signing_identity(certificate, key_entry):
    key = gopass(key_entry)
    pem = run("openssl", "x509", "-inform", "DER", "-in", certificate, "-outform", "PEM")
    cert_public = run("openssl", "x509", "-pubkey", "-noout", data=pem)
    if public_key(key) != public_key(cert_public, "-pubin"):
        raise ValueError("Certificate and gopass private key do not match")
    run("/usr/bin/security", "verify-cert", "-c", certificate, "-p", "codeSign")
    fingerprint = run("openssl", "x509", "-inform", "DER", "-in", certificate,
                      "-noout", "-fingerprint", "-sha1").decode().strip().split("=", 1)[1].replace(":", "")
    password = secrets.token_urlsafe(48).encode()
    keychain_password = secrets.token_urlsafe(48)
    with input_pipe(key) as key_fd, input_pipe(password + b"\n") as password_fd:
        # Keychain's PKCS#12 importer requires these legacy transport algorithms.
        # This short-lived bundle has an independent random 384-bit password.
        bundle = run("openssl", "pkcs12", "-export", "-macalg", "sha1",
                     "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-inkey", f"/dev/fd/{key_fd}",
                     "-passout", f"fd:{password_fd}", data=pem, sensitive=True,
                     pass_fds=(key_fd, password_fd))
    with tempfile.TemporaryDirectory(prefix="ratatoskr-signing-") as directory:
        root = Path(directory)
        keychain = root / "publisher.keychain-db"
        encrypted_bundle = root / "publisher.p12"
        try:
            with os.fdopen(os.open(encrypted_bundle, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
                file.write(bundle)
            security("create-keychain", "-p", keychain_password, keychain)
            keychain.chmod(0o600)
            security("set-keychain-settings", "-lut", "3600", keychain)
            security("unlock-keychain", "-p", keychain_password, keychain)
            security("import", encrypted_bundle, "-k", keychain, "-P", password.decode(),
                     "-T", "/usr/bin/codesign")
            security("set-key-partition-list", "-S", "apple-tool:,apple:,codesign:",
                     "-s", "-k", keychain_password, keychain)
            encrypted_bundle.unlink()
            identities = run("/usr/bin/security", "find-identity", "-v", "-p", "codesigning", keychain).decode()
            if fingerprint not in identities or "Developer ID Application:" not in identities:
                raise ValueError("Temporary keychain lacks the expected Developer ID identity")
            yield keychain, fingerprint
        finally:
            if keychain.exists():
                run("/usr/bin/security", "delete-keychain", keychain)


def verify(path, identifier, team):
    if not re.fullmatch(r"[A-Z0-9]{10}", team or ""):
        raise ValueError("Invalid signing team")
    if not re.fullmatch(r"[A-Za-z0-9.-]+", identifier):
        raise ValueError("Invalid signing identifier")
    requirement = (
        f'anchor apple generic and identifier "{identifier}" and certificate leaf[subject.OU] = "{team}" '
        'and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
        'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
    )
    run("/usr/bin/codesign", "--verify", "--deep", "--strict", "--test-requirement", "=" + requirement, path)
    result = subprocess.run(["/usr/bin/codesign", "--display", "--verbose=4", str(path)], capture_output=True)
    if result.returncode:
        raise ValueError("Cannot read code signature")
    info = result.stderr.decode(errors="replace")
    if "(runtime)" not in info or "Timestamp=" not in info:
        raise ValueError("Release requires hardened runtime and an Apple secure timestamp")
    print(f"Verified Developer ID publisher and identity: {identifier}")


def accepted(result):
    if result.get("status") != "Accepted":
        raise ValueError("Apple did not accept the notarization submission")


def sign(path, identifier, team, identity):
    keychain, fingerprint = identity
    run("/usr/bin/codesign", "--force", "--sign", fingerprint, "--keychain", keychain,
        "--identifier", identifier, "--options", "runtime", "--timestamp",
        "--preserve-metadata=entitlements", path)
    verify(path, identifier, team)


def notary_credentials(entry):
    value = json.loads(gopass(entry))
    if not re.fullmatch(r"[A-Z0-9]{10}", value.get("key_id", "")):
        raise ValueError("Invalid notarization key ID")
    if not re.fullmatch(r"[a-fA-F0-9-]{36}", value.get("issuer_id", "")):
        raise ValueError("Invalid notarization issuer ID")
    key = value.get("private_key", "").encode()
    if not key.startswith(b"-----BEGIN PRIVATE KEY-----"):
        raise ValueError("Notarization entry must contain a PKCS#8 private key")
    return value, key


def notary_command(entry, *args):
    metadata, key = notary_credentials(entry)
    tool = run("xcrun", "--find", "notarytool").decode().strip()
    # notarytool rejects both anonymous and named pipes. Its regular-file input
    # lives only inside a disposable AES-256 image, never on an unencrypted disk.
    root = Path(tempfile.mkdtemp(prefix="ratatoskr-notary-"))
    image = root / "credential.dmg"
    mount = root / "mounted"
    password = secrets.token_urlsafe(48).encode() + b"\n"
    try:
        run("/usr/sbin/diskutil", "image", "create", "blank", "--encrypt", "--stdinpassphrase",
            "--size", "64m", "--volumeName", "RatatoskrNotary", image, data=password, sensitive=True)
        image.chmod(0o600)
        run("/usr/sbin/diskutil", "image", "attach", "--stdinpassphrase", "--nobrowse",
            "--mountPoint", mount, image, data=password, sensitive=True)
        if not os.path.ismount(mount):
            raise ValueError("Encrypted notary volume was not mounted")
        mount.chmod(0o700)
        (mount / ".metadata_never_index").touch(mode=0o600)
        run("/usr/bin/xattr", "-w", "com.apple.metadata:com_apple_backup_excludeItem", "com.apple.backupd", mount)
        path = mount / "credential.p8"
        with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
            file.write(key)
        return subprocess.run([tool, *map(str, args), "--key", str(path),
                               "--key-id", metadata["key_id"], "--issuer", metadata["issuer_id"]],
                              capture_output=True, cwd=ROOT)
    finally:
        if os.path.ismount(mount):
            run("/usr/bin/hdiutil", "detach", mount)
        shutil.rmtree(root)


def notarize(path, entry, log_path):
    response = notary_command(entry, "submit", path, "--wait", "--timeout", "15m", "--output-format", "json")
    try:
        result = json.loads(response.stdout)
    except json.JSONDecodeError as error:
        raise ValueError("Apple returned no usable notarization status: " +
                         response.stderr.decode(errors="replace")) from error
    receipt = log_path.with_suffix(".submission.json")
    receipt.write_text(json.dumps({"artifact": Path(path).name,
                                   "sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest(),
                                   "submission": result}, indent=2) + "\n")
    print(f"Apple submission receipt: {receipt}")
    if result.get("status") in {"Invalid", "Rejected"} and result.get("id"):
        log = notary_command(entry, "log", result["id"])
        if log.returncode:
            raise ValueError("Notarization failed and its diagnostic log could not be retrieved")
        log_path.write_bytes(log.stdout)
        print(f"Notarization diagnostics: {log_path}", file=sys.stderr)
    accepted(result)
    if response.returncode:
        raise ValueError("Notarization command failed despite its returned status")


def validate_candidate_archive(path):
    with zipfile.ZipFile(path) as archive:
        for info in archive.infolist():
            name = info.filename
            if name.startswith("/") or ".." in name.split("/") or "\\" in name:
                raise ValueError("Unsafe candidate archive path")
            if not (name == APP or name.startswith(APP + "/") or name.startswith("__MACOSX/")):
                raise ValueError("Candidate archive contains files outside the app")
            if stat.S_ISLNK(info.external_attr >> 16):
                target = archive.read(info).decode()
                resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
                if target.startswith("/") or not resolved.startswith(APP + "/"):
                    raise ValueError("Candidate archive symlink escapes the app")


def validate_run(metadata, commit):
    if (metadata.get("event") != "workflow_dispatch" or metadata.get("headBranch") != "main"
            or metadata.get("headSha") != commit or metadata.get("conclusion") != "success"
            or metadata.get("workflowName") != "Build release candidates"):
        raise ValueError("Expected a successful main-branch candidate run for the checked-out commit")


def sign_update(path, version, args):
    env = os.environ.copy()
    env["TAURI_SIGNING_PRIVATE_KEY"] = gopass(args.update_key_entry).decode().strip()
    env["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] = gopass(args.update_password_entry).decode().rstrip("\r\n")
    run("node", ROOT / "desktop/node_modules/@tauri-apps/cli/tauri.js", "signer", "sign",
        "--app-version", version, path, env=env, sensitive=True)


def verify_updates(directory):
    verifier = ROOT / "target/debug/examples/verify_release_artifact"
    for signature in directory.glob("*.sig"):
        run(verifier, str(signature)[:-4], signature)


def release(args):
    if sys.platform != "darwin" or os.environ.get("GITHUB_ACTIONS"):
        raise ValueError("Apple signing must run on the local Mac, outside GitHub Actions")
    for tool in ["node", "cargo", "gh", "gopass", "openssl", "xcrun"]:
        if not shutil.which(tool):
            raise ValueError(f"Install {tool} and make it available on PATH before signing")
    if not (ROOT / "desktop/node_modules/@tauri-apps/cli/tauri.js").is_file():
        raise ValueError("Run npm ci in desktop before signing")
    if run("git", "status", "--porcelain").strip():
        raise ValueError("Release requires a clean checkout")
    commit = run("git", "rev-parse", "HEAD").decode().strip()
    remote_commit = run("git", "ls-remote", "origin", "refs/heads/main").decode().split()[0]
    if commit != remote_commit:
        raise ValueError("Check out the reviewed origin/main commit before signing")
    metadata = json.loads(run("gh", "run", "view", args.run_id, "--repo", REPOSITORY,
                             "--json", "event,headBranch,headSha,conclusion,workflowName"))
    validate_run(metadata, commit)
    version = json.loads((ROOT / "desktop/package.json").read_text())["version"]
    run("cargo", "build", "--locked", "-p", "ratatoskr-core", "--example", "verify_release_artifact")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    downloads = output / "candidates"
    artifacts = output / "artifacts"
    artifacts.mkdir()
    run("gh", "run", "download", args.run_id, "--repo", REPOSITORY, "--dir", downloads)
    windows = downloads / "windows-x86_64"
    for name in [f"Ratatoskr-Skill-Sync-{version}-windows-x86_64-setup.exe", "ratatoskr-windows-x86_64.exe"]:
        for suffix in ["", ".sig"]:
            shutil.copyfile(windows / (name + suffix), artifacts / (name + suffix))
    verify_updates(artifacts)
    with signing_identity(args.certificate, args.signing_key_entry) as identity:
        for platform, architecture in [("darwin-aarch64", "arm64"), ("darwin-x86_64", "x86_64")]:
            candidate = downloads / platform / f"Ratatoskr-candidate-{platform}.zip"
            validate_candidate_archive(candidate)
            destination = output / platform
            run("ditto", "-x", "-k", candidate, destination)
            app = destination / APP
            info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
            if info.get("CFBundleIdentifier") != BUNDLE_ID or info.get("CFBundleShortVersionString") != version:
                raise ValueError("Candidate bundle identity/version mismatch")
            helper = app / "Contents/MacOS/ratatoskr"
            for binary in [helper, app / "Contents/MacOS" / info["CFBundleExecutable"]]:
                if run("lipo", "-archs", binary).decode().split() != [architecture]:
                    raise ValueError("Candidate architecture mismatch")
            sign(helper, "ratatoskr", args.team_id, identity)
            sign(app, BUNDLE_ID, args.team_id, identity)
            app_zip = output / f"{platform}-notary.zip"
            run("ditto", "-c", "-k", "--keepParent", app, app_zip)
            notarize(app_zip, args.notary_entry, output / f"{platform}-notary-log.json")
            run("xcrun", "stapler", "staple", app)
            run("xcrun", "stapler", "validate", app)
            run("/usr/sbin/spctl", "--assess", "--type", "execute", "--verbose=2", app)
            archive = artifacts / f"Ratatoskr-Skill-Sync-{version}-{platform}.app.tar.gz"
            run("tar", "-czf", archive, "-C", destination, APP)
            extracted = output / f"{platform}-archive-check"
            extracted.mkdir()
            run("tar", "-xzf", archive, "-C", extracted)
            verify(extracted / APP, BUNDLE_ID, args.team_id)
            verify(extracted / APP / "Contents/MacOS/ratatoskr", "ratatoskr", args.team_id)
            run("xcrun", "stapler", "validate", extracted / APP)
            cli = artifacts / f"ratatoskr-{platform}"
            shutil.copyfile(helper, cli)
            cli.chmod(0o755)
            verify(cli, "ratatoskr", args.team_id)
            cli_zip = output / f"{platform}-cli-notary.zip"
            run("ditto", "-c", "-k", "--keepParent", cli, cli_zip)
            notarize(cli_zip, args.notary_entry, output / f"{platform}-cli-notary-log.json")
            for path in [archive, cli]:
                sign_update(path, version, args)
    verify_updates(artifacts)
    run("node", "scripts/release-manifest.mjs", artifacts, version, REPOSITORY)
    print(f"Verified signed release artifacts: {artifacts}")
    if args.draft:
        run("gh", "release", "create", f"v{version}", *sorted(artifacts.iterdir()),
            "--repo", REPOSITORY, "--draft", "--target", commit,
            "--title", f"Ratatoskr Skill Sync {version}", "--generate-notes")
        print(f"Created draft v{version}; publishing still requires owner approval")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("verify")
    check.add_argument("path", type=Path)
    check.add_argument("identifier")
    commands.add_parser("accepted").add_argument("result", type=Path)
    for name in ["release", "check-identity"]:
        command = commands.add_parser(name)
        command.add_argument("--certificate", type=Path, required=True)
        command.add_argument("--signing-key-entry", required=True)
        command.add_argument("--team-id", required=True)
        if name == "release":
            command.add_argument("--run-id", required=True)
            command.add_argument("--notary-entry", required=True)
            command.add_argument("--update-key-entry", required=True)
            command.add_argument("--update-password-entry", required=True)
            command.add_argument("--output", type=Path, required=True)
            command.add_argument("--draft", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "verify":
            verify(args.path, args.identifier, os.environ.get("APPLE_TEAM_ID", ""))
        elif args.command == "accepted":
            accepted(json.loads(args.result.read_text()))
        elif args.command == "check-identity":
            with tempfile.TemporaryDirectory(prefix="ratatoskr-publisher-probe-") as directory:
                source = Path(directory) / "probe.c"
                binary = Path(directory) / "probe"
                source.write_text("int main(void) { return 0; }\n")
                run("xcrun", "clang", source, "-o", binary)
                with signing_identity(args.certificate, args.signing_key_entry) as identity:
                    sign(binary, "ratatoskr-signing-probe", args.team_id, identity)
        else:
            release(args)
    except (ValueError, OSError, KeyError, zipfile.BadZipFile) as error:
        parser.exit(1, f"{error}\n")
