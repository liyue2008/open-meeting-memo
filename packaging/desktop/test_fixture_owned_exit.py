"""Original-Popen failure cleanup, with synthetic stdin-controlled children only."""
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('fixture_runner', HERE / 'fixture_runner.py')
assert spec is not None
runner = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(runner)

class OwnedExitTests(unittest.TestCase):
    def child(self):
        return subprocess.Popen([sys.executable, '-u', '-c', 'import sys; print("ready",flush=True); assert sys.stdin.readline()=="quit\\n"; print("done",flush=True)'], stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def test_bridge_failure_normal_quit_waits_original_handle_and_drains(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d); binary = pathlib.Path(sys.executable).resolve()
            (root/'bridge-result.json').write_text(json.dumps({'result':{'ok':False,'error':'synthetic failure'}}))
            child = self.child(); assert child.stdin is not None and child.stdout is not None; self.assertEqual(child.stdout.readline(),b'ready\n')
            def quit_owned(*args):
                child.stdin.write(b'quit\n'); child.stdin.flush(); return True
            try:
                with patch.object(runner, 'request_normal_exit', side_effect=quit_owned):
                    code, waited = runner.await_owned(child,1,root,'synthetic identity',binary)
                self.assertEqual((code,waited),(1,True))
                self.assertEqual(child.stdout.read(),b'done\n')  # actual EOF, not PID disappearance
                receipt=json.loads((root/'owned-process.json').read_text())
                self.assertTrue(receipt['waited']); self.assertEqual(receipt['exitCode'],0)
                self.assertEqual(receipt['failureExitCode'],1); self.assertEqual(receipt['forcedSignals'],0)
            finally:
                if child.poll() is None: child.stdin.write(b'quit\n'); child.stdin.flush()
                child.wait(); child.stdin.close(); child.stdout.close()

    def test_refused_normal_quit_retains_original_live_child(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d); child=self.child(); assert child.stdin is not None and child.stdout is not None; self.assertEqual(child.stdout.readline(),b'ready\n')
            (root/'bridge-result.json').write_text('{"result":{"ok":false}}')
            try:
                with patch.object(runner,'request_normal_exit',return_value=False):
                    self.assertEqual(runner.await_owned(child,1,root,'synthetic',pathlib.Path(sys.executable)),(1,False))
                self.assertIsNone(child.poll()); receipt=json.loads((root/'owned-process.json').read_text());self.assertFalse(receipt['waited']);self.assertEqual(receipt['forcedSignals'],0)
            finally:
                child.stdin.write(b'quit\n');child.stdin.flush();child.wait();self.assertEqual(child.stdout.read(),b'done\n');child.stdin.close();child.stdout.close()

    def test_timeout_normal_quit_preserves_timeout_and_waits(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d); child=self.child(); assert child.stdin is not None and child.stdout is not None
            self.assertEqual(child.stdout.readline(),b'ready\n')
            def quit_owned(*args):
                child.stdin.write(b'quit\n');child.stdin.flush();return True
            try:
                with patch.object(runner,'request_normal_exit',side_effect=quit_owned):
                    self.assertEqual(runner.await_owned(child,0,root,'synthetic',pathlib.Path(sys.executable)),(124,True))
                self.assertEqual(child.stdout.read(),b'done\n');self.assertEqual(child.wait(),0)
                receipt=json.loads((root/'owned-process.json').read_text());self.assertTrue(receipt['timeout']);self.assertTrue(receipt['waited'])
            finally:
                if child.poll() is None: child.stdin.write(b'quit\n');child.stdin.flush()
                child.wait();child.stdin.close();child.stdout.close()

    def test_identity_mismatch_retains_real_child_without_cocoa(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);child=self.child();assert child.stdin is not None and child.stdout is not None
            self.assertEqual(child.stdout.readline(),b'ready\n');(root/'bridge-result.json').write_text('{"result":{"ok":false}}')
            try:
                with patch.object(runner.subprocess,'run',side_effect=AssertionError('must not invoke Cocoa')):
                    self.assertEqual(runner.await_owned(child,1,root,'unbound identity',pathlib.Path(sys.executable)),(1,False))
                self.assertIsNone(child.poll())
            finally:
                child.stdin.write(b'quit\n');child.stdin.flush();child.wait();self.assertEqual(child.stdout.read(),b'done\n');child.stdin.close();child.stdout.close()

    def test_bound_uid_start_path_and_hash_before_normal_quit(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as d:
            binary=pathlib.Path(d)/'synthetic-host';binary.write_bytes(b'not executed')
            child=SimpleNamespace(pid=12345,poll=lambda:None)
            identity=f'12345 4 {runner.os.getuid()} Mon Oct 19 13:00:00 2026 {binary}'
            ps=subprocess.CompletedProcess([],0,identity+'\n','')
            accepted=subprocess.CompletedProcess([],0,'accepted\n','')
            with patch.object(runner.subprocess,'run',side_effect=[ps,accepted]) as calls:
                self.assertTrue(runner.request_normal_exit(child,identity,binary,runner.sha(binary)))
                self.assertEqual(calls.call_count,2)
                self.assertEqual(calls.call_args_list[1].args[0][0],'/usr/bin/osascript')
            with patch.object(runner.subprocess,'run',side_effect=AssertionError('changed hash must retain')):
                self.assertFalse(runner.request_normal_exit(child,identity,binary,'not-the-start-hash'))
            wrong_uid=identity.replace(f'12345 4 {runner.os.getuid()} ',f'12345 4 {runner.os.getuid()+1} ',1)
            wrong_path=identity.replace(str(binary),str(binary)+'-other')
            with patch.object(runner.subprocess,'run',side_effect=AssertionError('UID/path mismatch must retain')):
                for other in [wrong_uid,wrong_path]:
                    self.assertFalse(runner.request_normal_exit(child,other,binary,runner.sha(binary)))
            changed=subprocess.CompletedProcess([],0,identity.replace('13:00:00','13:01:00')+'\n','')
            with patch.object(runner.subprocess,'run',return_value=changed) as calls:
                self.assertFalse(runner.request_normal_exit(child,identity,binary,runner.sha(binary)))
                self.assertEqual(calls.call_count,1)

    def test_exception_uses_normal_quit_and_original_wait(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);child=self.child();assert child.stdin is not None and child.stdout is not None
            self.assertEqual(child.stdout.readline(),b'ready\n');(root/'bridge-result.json').write_text('{"result":[]}')
            def quit_owned(*args):
                child.stdin.write(b'quit\n');child.stdin.flush();return True
            try:
                with patch.object(runner,'request_normal_exit',side_effect=quit_owned):
                    self.assertEqual(runner.await_owned(child,1,root,'synthetic',pathlib.Path(sys.executable)),(1,True))
                self.assertEqual(child.stdout.read(),b'done\n');self.assertEqual(child.wait(),0)
            finally:
                if child.poll() is None:child.stdin.write(b'quit\n');child.stdin.flush()
                child.wait();child.stdin.close();child.stdout.close()

    def test_failed_bridge_already_exited_child_uses_original_wait(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);child=self.child();assert child.stdin is not None and child.stdout is not None
            self.assertEqual(child.stdout.readline(),b'ready\n');child.stdin.write(b'quit\n');child.stdin.flush()
            self.assertEqual(child.stdout.read(),b'done\n')
            (root/'bridge-result.json').write_text('{"result":{"ok":false}}')
            try:
                with patch.object(runner,'request_normal_exit',side_effect=AssertionError('already exited')):
                    self.assertEqual(runner.await_owned(child,1,root,'synthetic',pathlib.Path(sys.executable)),(1,True))
                self.assertEqual(child.wait(),0)
            finally:
                child.wait();child.stdin.close();child.stdout.close()

    def test_identity_mismatch_never_requests_cocoa_quit(self):
        with patch.object(runner.subprocess,'run',side_effect=AssertionError('must not invoke Cocoa')):
            self.assertFalse(runner.request_normal_exit(None,'unbound identity',pathlib.Path(sys.executable)))

if __name__=='__main__': unittest.main()
