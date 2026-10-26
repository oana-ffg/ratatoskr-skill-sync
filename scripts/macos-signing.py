#!/usr/bin/env python3
"""Ephemeral CI credentials and strict publisher verification. Never prints secrets."""
import base64
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, **kwargs)


def required(name):
    value = os.environ.get(name, "")
    if not value:
        raise ValueError(f"Configure {name} before building a publisher-signed release")
    return value


def credential_paths():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_OS") != "macOS":
        raise ValueError("Credential installation is restricted to disposable macOS Actions runners")
    root = Path(required("RUNNER_TEMP")).resolve()
    return root / "ratatoskr-signing.keychain-db", root / "ratatoskr-notary.p8"


def keychains():
    return shlex.split(run("/usr/bin/security", "list-keychains", "-d", "user", text=True).stdout)


def prepare():
    keychain, notary = credential_paths()
    if keychain.exists() or notary.exists():
        raise ValueError("Refusing to overwrite existing runner credentials")
    certificate = base64.b64decode(required("APPLE_CERTIFICATE"), validate=True)
    password = required("KEYCHAIN_PASSWORD")
    certificate_password = required("APPLE_CERTIFICATE_PASSWORD")
    notary_key = required("APPLE_API_KEY_CONTENT").encode()
    if not notary_key.startswith(b"-----BEGIN PRIVATE KEY-----"):
        raise ValueError("APPLE_API_KEY_CONTENT must be the downloaded notarization API key")
    fd, certificate_path = tempfile.mkstemp(prefix="ratatoskr-certificate-", dir=keychain.parent)
    try:
        with os.fdopen(fd, "wb") as file:
            file.write(certificate)
        # Passwords are masked Actions secrets; the runner and its keychain are disposable.
        run("/usr/bin/security", "create-keychain", "-p", password, str(keychain))
        run("/usr/bin/security", "set-keychain-settings", "-lut", "21600", str(keychain))
        run("/usr/bin/security", "unlock-keychain", "-p", password, str(keychain))
        run("/usr/bin/security", "import", certificate_path, "-k", str(keychain),
            "-P", certificate_password, "-T", "/usr/bin/codesign")
        run("/usr/bin/security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:",
            "-s", "-k", password, str(keychain))
        run("/usr/bin/security", "list-keychains", "-d", "user", "-s", *keychains(), str(keychain))
        with os.fdopen(os.open(notary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
            file.write(notary_key)
        with open(required("GITHUB_ENV"), "a") as file:
            file.write(f"APPLE_API_KEY_PATH={notary}\n")
    finally:
        Path(certificate_path).unlink()


def cleanup():
    keychain, notary = credential_paths()
    notary.unlink(missing_ok=True)
    if keychain.exists():
        run("/usr/bin/security", "delete-keychain", str(keychain))


def verify(path, identifier):
    team = required("APPLE_TEAM_ID")
    if not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("Invalid APPLE_TEAM_ID")
    if not re.fullmatch(r"[A-Za-z0-9.-]+", identifier):
        raise ValueError("Invalid signing identifier")
    requirement = (
        f'anchor apple generic and identifier "{identifier}" and certificate leaf[subject.OU] = "{team}" '
        'and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
        'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
    )
    run("/usr/bin/codesign", "--verify", "--deep", "--strict", "--test-requirement", "=" + requirement, path)
    info = run("/usr/bin/codesign", "--display", "--verbose=4", path, text=True).stderr
    if "(runtime)" not in info or "Timestamp=" not in info:
        raise ValueError("Release requires hardened runtime and an Apple secure timestamp")
    print(f"Verified Developer ID publisher and identity: {identifier}")


def main():
    if sys.argv[1:] == ["prepare"]:
        prepare()
    elif sys.argv[1:] == ["cleanup"]:
        cleanup()
    elif len(sys.argv) == 4 and sys.argv[1] == "verify":
        verify(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 3 and sys.argv[1] == "accepted":
        status = json.loads(Path(sys.argv[2]).read_text())["status"]
        if status != "Accepted":
            raise ValueError("Apple did not accept the standalone helper notarization")
        print("Apple accepted the standalone helper notarization")
    else:
        raise ValueError("Usage: macos-signing.py prepare | cleanup | verify PATH IDENTIFIER | accepted JSON")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        # Commands contain credentials. Never render argv or captured subprocess output.
        sys.exit(f"macOS signing command failed (exit {error.returncode})")
    except (ValueError, OSError) as error:
        sys.exit(str(error))
