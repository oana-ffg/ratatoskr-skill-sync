"""Execute base and head CLIs with identical inputs and compare plist bytes."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="ratatoskr-plist-") as directory:
    temporary = Path(directory).resolve()
    baseline = temporary / "baseline"
    subprocess.run(["git", "worktree", "add", "--detach", str(baseline), os.environ["BASE_SHA"]], cwd=root, check=True)
    try:
        target = temporary / "baseline-target"
        subprocess.run(["cargo", "build", "--locked", "-p", "ratatoskr-skill-sync"], cwd=baseline,
                       env={**os.environ, "CARGO_TARGET_DIR": str(target)}, check=True)
        source = temporary / "source & unicode Æ"
        source.mkdir()
        config = temporary / "config.toml"
        config.write_text(f"source = {json.dumps(str(source))}\nmachine = 'plist-ci'\nstate_dir = {json.dumps(str(temporary / 'state'))}\n")
        current = root / "target/debug/ratatoskr"
        # cargo test builds the integration-test CLI at this exact path.
        for interval in (60, 120, 3600):
            args = ["--config", str(config), "schedule", "plan", "--helper", str(current), "--seconds", str(interval)]
            old = subprocess.check_output([str(target / "debug/ratatoskr"), *args], cwd=root)
            new = subprocess.check_output([str(current), *args], cwd=root)
            if old != new:
                raise AssertionError(f"launchd plist bytes changed at interval {interval}")
        print("PASS: launchd plist bytes identical to PR base at 60/120/3600 seconds")
    finally:
        subprocess.run(["git", "worktree", "remove", str(baseline)], cwd=root, check=True)
