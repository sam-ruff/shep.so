#!/usr/bin/env python3
"""Native repeated opening of image-heavy mail, including final image pixels."""
import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path
from e2e import McpClient, click, check, wait, shot, mail_row_y, ROOT


def opened(mcp, index):
    mcp.batch(click(400, mail_row_y(index)), check('selected', ['Dispatch update', 'Delivery update'][index]),
              check('html_view_current', True), check('html_loaded_images', 12),
              check('remote_image_pending', 0), check('html_rendered_images', 12), wait(250))


def measure(samples):
    fingerprint = hashlib.sha256((ROOT/'target/test-ui/shep').read_bytes()).hexdigest()
    mcp = McpClient()
    readings = []
    try:
        evidence = mcp.call('desktop.start', html_mail=True)['artifacts']
        mcp.batch(click(85, 355), check('selected', 'Dispatch update'), check('html_view_current', True),
                  click(1115, 376), check('images_allowed', True), check('html_loaded_images', 12))
        # The permission is per message; give the second its own explicit grant.
        mcp.batch(click(400, mail_row_y(1)), check('selected', 'Delivery update'), check('html_view_current', True))
        if not mcp.call('desktop.state')['images_allowed']:
            mcp.batch(click(1115, 376), check('images_allowed', True))
        references = []
        for index in range(2):
            opened(mcp, index)
            mcp.batch(shot(f'image-heavy-reference-{index}'))
            references.append(mcp.batch({'type': 'pixel_reference'})['actions'][0]['result']['points'])
        for cycle in range(samples):
            for index in range(2):
                result = mcp.batch({'type': 'measure_pixels', 'x': 400, 'y': mail_row_y(index),
                                    'points': references[index], 'timeout_ms': 5000},
                                   check('html_view_current', True))
                reading = {'case': ['warm_images_return', 'warm_images_repeat'][index], 'cycle': cycle, 'message': index, **result['actions'][0]['result'],
                           'cache_hits': result['state']['html_cache_hits']}
                readings.append(reading)
                print(json.dumps(reading), flush=True)
                mcp.batch(check('html_rendered_images', 12), check('html_loaded_images', 12), check('remote_image_pending', 0), wait(250))
        values = sorted(r['input_to_pixels_ms'] for r in readings)
        if hashlib.sha256((ROOT/'target/test-ui/shep').read_bytes()).hexdigest() != fingerprint:
            raise RuntimeError('The test binary changed during measurement.')
        return {'binary_sha256': fingerprint, 'artifacts': evidence, 'readings': readings,
                'summary': {'count': len(values), 'p50_ms': statistics.median(values),
                            'p95_ms': values[math.ceil(len(values)*.95)-1], 'max_ms': max(values)}}
    finally:
        mcp.close()

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--samples', type=int, default=20)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.samples <= 100:
        parser.error('samples must be 1–100')
    result = measure(args.samples)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result['summary']))
