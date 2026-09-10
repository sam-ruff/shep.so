#!/usr/bin/env python3
"""Stage separated public promo and protected browser assets. Never deploy."""
import argparse
from pathlib import Path
import shutil


def assemble(website, web, output):
    website, web, output = (Path(p).resolve() for p in (website, web, output))
    for source in (website, web):
        if not (source / 'index.html').is_file():
            raise ValueError(f'Missing built index.html: {source}')
        if output == source or output in source.parents or source in output.parents:
            raise ValueError('Output must be separate from source builds')
        for p in source.rglob('*'):
            if p.is_symlink():
                raise ValueError('Static build must not contain symlinks')
    if (web / 'preview.html').exists():
        raise ValueError('Refusing to stage fictional browser preview as production')
    if output.exists():
        raise ValueError('Use a fresh output directory; existing staged evidence is preserved')
    output.mkdir(parents=True)
    shutil.copytree(website, output / 'website')
    shutil.copytree(web, output / 'web')
    (output / 'website' / '.nojekyll').touch()
    print(f'Staged assets at {output}; not deployed. Provider parity remains a prerequisite.')

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--website', required=True)
    parser.add_argument('--web', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    assemble(args.website, args.web, args.output)
