from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "check_server_tester", ROOT / "scripts/check_server_tester.py"
)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = module
SPEC.loader.exec_module(module)

MINIMAL_RUNTIME_PACKAGES = (
    "alpine-baselayout",
    "alpine-baselayout-data",
    "alpine-keys",
    "alpine-release",
    "busybox",
    "busybox-binsh",
    "ca-certificates-bundle",
    "musl",
    "musl-utils",
    "scanelf",
)


def installed_db(*packages: str) -> str:
    """Render a minimal Alpine /lib/apk/db/installed with one record per package."""
    return "".join(f"C:Q1fake=\nP:{name}\nV:1.0-r0\nA:x86_64\n\n" for name in packages)


class TesterRuntimeDockerfileContractTests(unittest.TestCase):
    def test_repository_dockerfile_satisfies_static_contracts(self) -> None:
        self.assertEqual(module.static_errors(), [])

    def test_runtime_stage_that_installs_openssl_backed_ca_package_is_rejected(self) -> None:
        dockerfile = (
            "FROM alpine:3.22 AS tester-runtime\n"
            "RUN apk upgrade --no-cache \\\n"
            "    && apk add --no-cache ca-certificates\n"
        )
        errors = module.tester_runtime_errors(dockerfile)
        self.assertTrue(any("ca-certificates" in error for error in errors), errors)
        self.assertTrue(any("apk del" in error for error in errors), errors)

    def test_runtime_stage_comments_do_not_trigger_the_ca_package_rule(self) -> None:
        dockerfile = (
            "FROM alpine:3.22 AS tester-runtime\n"
            "# The full ca-certificates package links libcrypto3, so keep only the bundle.\n"
            "RUN apk upgrade --no-cache \\\n"
            "    && apk add --no-cache ca-certificates-bundle \\\n"
            "    && apk del --no-cache --purge apk-tools ssl_client\n"
        )
        self.assertEqual(module.tester_runtime_errors(dockerfile), [])


class TesterRuntimeImageContractTests(unittest.TestCase):
    def test_minimal_runtime_with_ca_bundle_is_accepted(self) -> None:
        self.assertEqual(
            module.image_errors(installed_db(*MINIMAL_RUNTIME_PACKAGES), ca_bundle_present=True),
            [],
        )

    def test_openssl_and_package_manager_packages_are_rejected(self) -> None:
        packages = MINIMAL_RUNTIME_PACKAGES + ("apk-tools", "libapk2", "ssl_client", "libssl3", "libcrypto3")
        errors = module.image_errors(installed_db(*packages), ca_bundle_present=True)
        for name in ("apk-tools", "libapk2", "ssl_client", "libssl3", "libcrypto3"):
            self.assertTrue(any(name in error for error in errors), (name, errors))

    def test_missing_ca_bundle_package_is_rejected(self) -> None:
        packages = tuple(name for name in MINIMAL_RUNTIME_PACKAGES if name != "ca-certificates-bundle")
        errors = module.image_errors(installed_db(*packages), ca_bundle_present=True)
        self.assertTrue(any("ca-certificates-bundle" in error for error in errors), errors)

    def test_missing_ca_bundle_file_is_rejected(self) -> None:
        errors = module.image_errors(installed_db(*MINIMAL_RUNTIME_PACKAGES), ca_bundle_present=False)
        self.assertTrue(any("/etc/ssl/certs/ca-certificates.crt" in error for error in errors), errors)

    def test_empty_package_database_is_rejected(self) -> None:
        errors = module.image_errors("", ca_bundle_present=True)
        self.assertTrue(errors)


if __name__ == "__main__":
    unittest.main()
