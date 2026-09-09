#!/usr/bin/env python3
"""Native cold and visited opens of a fictional sixteen-level table template."""
import hashlib
import json
from e2e import McpClient, click, check, wait, shot, mail_row_y, ROOT


def measure(samples):
    fingerprint = hashlib.sha256((ROOT/'target/test-ui/shep').read_bytes()).hexdigest()
    mcp = McpClient()
    evidence, readings = [], []
    try:
        evidence.append(mcp.call('desktop.start', html_mail=True)['artifacts'])
        mcp.batch(click(85,355), check('folder','Sent'), click(400,mail_row_y(4)),
                  check('selected','Deeply nested delivery'), check('html_view_current',True),
                  wait(250), shot('deep-table-reference'))
        points = mcp.batch({'type':'pixel_reference'})['actions'][0]['result']['points']
        for cycle in range(samples):
            evidence.append(mcp.call('desktop.start', html_mail=True)['artifacts'])
            mcp.batch(click(85,355), check('selected','Dispatch update'),
                      check('html_view_current',True), wait(300))
            state = mcp.call('desktop.state')
            if any('template-4' in identity for identity in state.get('html_cache_ids', [])):
                raise AssertionError('The cold deep template was already prepared')
            for case in ['cold_nested_table','warm_nested_table']:
                result = mcp.batch({'type':'measure_pixels','x':400,'y':mail_row_y(4),
                                    'points':points,'timeout_ms':5000},
                                   check('selected','Deeply nested delivery'),
                                   check('html_view_current',True))
                reading = {'case':case, 'cycle':cycle, **result['actions'][0]['result'],
                           'html_cache_hits':result['state']['html_cache_hits'], 'artifacts':evidence[-1]}
                readings.append(reading)
                print(json.dumps(reading), flush=True)
                mcp.batch(wait(200),click(400,mail_row_y(0)),check('selected','Dispatch update'),
                          check('html_view_current',True),wait(200))
        if hashlib.sha256((ROOT/'target/test-ui/shep').read_bytes()).hexdigest() != fingerprint:
            raise RuntimeError('The binary changed during measurement')
        return {'binary_sha256':fingerprint, 'evidence':evidence, 'readings':readings}
    finally:
        mcp.close()
