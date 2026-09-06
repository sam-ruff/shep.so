#!/usr/bin/env python3
"""Fail a coordinated release until all named prerequisites have evidence."""
import json
from pathlib import Path

REQUIRED = ('desktop_mobile_browser_parity', 'provider_contracts_and_recovery',
            'android_and_apple_execution', 'android_and_apple_distribution_signing',
            'beta_gateway_vps_verification', 'coordinated_artifact_publication')


def validate(data):
    if not isinstance(data, dict) or data.get('schema') != 1:
        raise ValueError('Invalid release-readiness schema')
    missing = [key for key in REQUIRED if data.get(key) is not True]
    if missing:
        raise ValueError('Coordinated release is not ready: ' + ', '.join(missing))


if __name__ == '__main__':
    root = Path(__file__).resolve().parents[2]
    try:
        validate(json.loads((root / 'shared/release-readiness.json').read_text()))
    except ValueError as error:
        raise SystemExit(str(error)) from None
    print('Coordinated release prerequisites recorded; normal quality/signing gates still apply.')
