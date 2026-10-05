"""Run the iced_test desktop suite against owned, disposable mail servers."""

import argparse
import datetime
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("filter", nargs="?", default="", help="Scenario name substring")
    parser.add_argument("--profile", default="test-ui", choices=("test-ui", "dev"))
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("This server runner currently supports Linux with Docker and OpenSSL.")
    for executable in ("cargo", "docker", "openssl"):
        if shutil.which(executable) is None:
            parser.error(f"Required executable is missing: {executable}")
    subprocess.run(["docker", "info", "--format", "{{.ServerVersion}}"], check=True)
    root = Path(__file__).resolve().parent.parent
    environment = os.environ.copy()
    # Set trust before Rust starts; never mutate process-wide trust in threaded tests.
    environment["SSL_CERT_FILE"] = str(root / "shared/mail-core/tests/fixtures/tls-cert.pem")
    environment["ICED_TEST_BACKEND"] = "tiny-skia"
    environment["CARGO_BUILD_JOBS"] = "4"
    command = [
        "cargo", "test", "-p", "shep", "--lib", "--all-features",
        "--profile", args.profile,
        f"ui::simulator_tests::real_mail::scenarios::{args.filter}",
        "--", "--ignored", "--test-threads=1", "--nocapture",
    ]
    log_dir = root / "artifacts/logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    log_path = log_dir / f"real-mail-{stamp}.log"
    print(f"Log: {log_path}", flush=True)
    with log_path.open("w") as log:
        result = subprocess.run(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT, check=False)
    output = log_path.read_text()
    print(output, end="")
    if result.returncode == 0 and not re.search(r"test result: ok\. [1-9][0-9]* passed", output):
        print("No real-server scenarios ran. Check the scenario filter.", file=sys.stderr)
        return 1
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
