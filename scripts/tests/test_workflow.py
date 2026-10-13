"""Keep source tags separate from the opt-in installer release workflow."""
from pathlib import Path
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]


class WorkflowTests(unittest.TestCase):
    def test_installer_requires_manual_dispatch_and_checks_out_requested_tag(self):
        workflow = yaml.load((ROOT / '.github/workflows/release.yml').read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(set(workflow['on']), {'workflow_dispatch'})
        self.assertEqual(workflow['on']['workflow_dispatch']['inputs']['release_tag']['required'], 'true')
        job = workflow['jobs']['release']
        self.assertEqual(job['env']['RELEASE_TAG'], '${{ inputs.release_tag }}')
        checkout = next(step for step in job['steps'] if step.get('uses', '').startswith('actions/checkout@'))
        self.assertEqual(checkout['with']['ref'], 'refs/tags/${{ inputs.release_tag }}')

    def test_release_rejects_non_version_tag_before_building(self):
        workflow = yaml.load((ROOT / '.github/workflows/release.yml').read_text(), Loader=yaml.BaseLoader)
        steps = workflow['jobs']['release']['steps']
        validation = next((s for s in steps if s.get('name') == 'Validate requested release tag'), None)
        self.assertIsNotNone(validation)
        import os
        import subprocess
        for tag, accepted in [('v0.1.5', True), ('main', False), ('v0.1.5; touch /tmp/unwanted', False), ('', False)]:
            result = subprocess.run(['bash', '-c', validation['run']], env=os.environ | {'RELEASE_TAG': tag}, capture_output=True)
            self.assertEqual(result.returncode == 0, accepted, tag)
