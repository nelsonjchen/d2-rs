// Copied only into the pinned instrumentation checkout by cli-scoreboard.py.
// The Go test bodies and assertions are the oracle; this is test plumbing only.
package e2etests_cli

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"testing"

	"github.com/d2lang/d2/lib/version"
	"github.com/d2lang/util-go/assert"
	"github.com/d2lang/util-go/xmain"
)

// Match the existing e2e scoreboard's generator-version allowance, only at
// golden assertions. Actual captured files/stdout and flag assertions stay raw.
var d2rsCLIVersion = regexp.MustCompile(`(data-d2-version|d2Version)="[^"]*"`)

func d2rsCLIGoldenBytes(data []byte) []byte {
	if os.Getenv("D2RS_CLI_BINARY") == "" {
		return data
	}
	return d2rsCLIVersion.ReplaceAll(data, []byte(`${1}="`+version.Version+`"`))
}

func d2rsCLITestdata(t *testing.T, extension string, data []byte) {
	if extension == ".svg" {
		data = d2rsCLIGoldenBytes(data)
	}
	assert.Testdata(t, extension, data)
}

func d2rsCLITestdataDir(t *testing.T, dir string) {
	if os.Getenv("D2RS_CLI_BINARY") == "" {
		assert.TestdataDir(t, dir)
		return
	}
	normalized := t.TempDir()
	// Generated-file hashes for the version allowance below: original and
	// normalized bytes, keyed by slash path relative to dir.
	rehashed := map[string][2]string{}
	err := filepath.WalkDir(dir, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		relative, err := filepath.Rel(dir, path)
		if err != nil {
			return err
		}
		target := filepath.Join(normalized, relative)
		if entry.IsDir() {
			return os.MkdirAll(target, 0755)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if filepath.Ext(path) == ".svg" {
			original := sha256.Sum256(data)
			data = d2rsCLIGoldenBytes(data)
			current := sha256.Sum256(data)
			rehashed[filepath.ToSlash(relative)] = [2]string{hex.EncodeToString(original[:]), hex.EncodeToString(current[:])}
		}
		return os.WriteFile(target, data, 0644)
	})
	assert.Success(t, err)
	assert.Success(t, d2rsCLIRehashBoardManifests(normalized, rehashed))
	assert.TestdataDir(t, normalized)
}

// d2cli/board_output_publish.go records each generated file's SHA-256 in
// .d2-board-output-manifest-v1.json. Apply the SVG version allowance to those
// digests too: an entry is rewritten only when it matches the native file's
// actual bytes, so a wrong digest or path still fails the golden comparison.
func d2rsCLIRehashBoardManifests(root string, rehashed map[string][2]string) error {
	return filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil || entry.IsDir() || entry.Name() != ".d2-board-output-manifest-v1.json" {
			return err
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		var manifest struct {
			Format  string `json:"format"`
			Version int    `json:"version"`
			Files   []struct {
				Path   string `json:"path"`
				SHA256 string `json:"sha256"`
			} `json:"files"`
			Directories []string `json:"directories"`
		}
		if err := json.Unmarshal(data, &manifest); err != nil {
			return nil
		}
		base, err := filepath.Rel(root, filepath.Dir(path))
		if err != nil {
			return err
		}
		for i, file := range manifest.Files {
			key := filepath.ToSlash(filepath.Join(base, filepath.FromSlash(file.Path)))
			if hashes, ok := rehashed[key]; ok && file.SHA256 == hashes[0] {
				manifest.Files[i].SHA256 = hashes[1]
			}
		}
		encoded, err := json.MarshalIndent(manifest, "", "  ")
		if err != nil {
			return err
		}
		encoded = append(encoded, '\n')
		if bytes.Equal(encoded, data) {
			return nil
		}
		return os.WriteFile(path, encoded, 0644)
	})
}

var d2rsCLICalls sync.Map
var d2rsCLIEmptyInputs sync.Map

func d2rsCLITempDir(t *testing.T) (string, func()) {
	root := os.Getenv("D2RS_CLI_WORK")
	if root == "" {
		return assert.TempDir(t)
	}
	// Go and Rust run sequentially. Reuse each case's input path so direct
	// stdout containing an absolute input filename remains byte-comparable.
	// Refuse stale files rather than silently deleting a previous binding.
	dir := filepath.Join(root, t.Name())
	if err := os.MkdirAll(filepath.Dir(dir), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(dir, 0755); err != nil {
		t.Fatal(err)
	}
	return dir, func() {
		if err := os.RemoveAll(dir); err != nil {
			t.Error(err)
		}
	}
}

func TestD2rsCLITempDir(t *testing.T) {
	t.Setenv("D2RS_CLI_WORK", t.TempDir())
	first, cleanup := d2rsCLITempDir(t)
	if err := os.WriteFile(filepath.Join(first, "input.d2"), []byte("a -> b"), 0644); err != nil {
		t.Fatal(err)
	}
	cleanup()
	second, cleanup := d2rsCLITempDir(t)
	defer cleanup()
	if first != second || !filepath.IsAbs(second) {
		t.Fatalf("case path changed: %q -> %q", first, second)
	}
	files, err := os.ReadDir(second)
	if err != nil || len(files) != 0 {
		t.Fatalf("previous run leaked files: %v, %v", files, err)
	}
}

func d2rsCLIInput(dir, path, data string) {
	if os.Getenv("D2RS_CLI_CAPTURE") == "" {
		return
	}
	value, _ := d2rsCLIEmptyInputs.LoadOrStore(dir, &sync.Map{})
	value.(*sync.Map).Store(filepath.ToSlash(path), data == "")
}

type d2rsCaptureWriter struct {
	io.WriteCloser
	captured bytes.Buffer
}

func d2rsCLIProcessError(stderr string, processErr error) error {
	// xmain.TestState.Wait adds the test program's name. Strip only the
	// native executable's transport prefix and preserve the diagnostic body.
	// mainFatal writes the error through cmdlog's "err: " line prefix;
	// warn/info/success lines are ordinary logs, not part of the error.
	var lines []string
	for _, line := range strings.Split(strings.TrimSuffix(stderr, "\n"), "\n") {
		if line == "err:" {
			lines = append(lines, "")
		} else if strings.HasPrefix(line, "err: ") {
			lines = append(lines, strings.TrimPrefix(line, "err: "))
		}
	}
	// xmain.mainFatal prints one more line after a UsageError's message;
	// it is not part of the error the in-process test receives.
	if n := len(lines); n > 1 && lines[n-1] == "Run with --help to see usage." {
		for _, line := range lines[:n-1] {
			if strings.Contains(line, "bad usage: ") {
				lines = lines[:n-1]
				break
			}
		}
	}
	message := strings.Join(lines, "\n")
	if message == "" {
		return processErr
	}
	return errors.New(message)
}

// A subprocess's mainFatal projection loses typed ExitError context. The
// optional native record preserves the original API error; never infer it
// from a case name or manufacture an expected wrapper in this adapter.
func d2rsCLIRecordedError(stderr string, processErr error, data []byte) error {
	terminal := d2rsCLIProcessError(stderr, processErr)
	if len(data) == 0 {
		return terminal // historical runners predate the optional transport
	}
	var record struct {
		Kind    string `json:"kind"`
		Error   string `json:"error"`
		Message string `json:"message"`
		Code    int    `json:"code"`
	}
	if err := json.Unmarshal(data, &record); err != nil {
		return fmt.Errorf("invalid native API error record: %w", err)
	}
	if record.Kind != "exit" {
		return terminal
	}
	var exited *exec.ExitError
	if !errors.As(processErr, &exited) || exited.ExitCode() != record.Code || terminal.Error() != record.Message || record.Error == "" {
		return errors.New("native API error record disagrees with terminal error")
	}
	return errors.New(record.Error)
}

func TestD2rsCLIProcessError(t *testing.T) {
	const detail = "failed to compile input.d2: render target \"b\" not found\nsecond line"
	tms := &xmain.TestState{
		Args: []string{"test-d2"},
		Run: func(context.Context, *xmain.State) error {
			return d2rsCLIProcessError("err: "+strings.ReplaceAll(detail, "\n", "\nerr: ")+"\n", errors.New("exit status 1"))
		},
	}
	ctx := context.Background()
	tms.Start(t, ctx)
	defer tms.Cleanup(t)
	err := tms.Wait(ctx)
	if err == nil || err.Error() != "failed to wait xmain test: test-d2: "+detail {
		t.Fatalf("diagnostic transport changed: %v", err)
	}
	processErr := errors.New("exit status 1")
	if d2rsCLIProcessError("", processErr) != processErr {
		t.Fatal("missing diagnostic lost its process error")
	}
	usage := &xmain.TestState{
		Args: []string{"test-d2"},
		Run: func(context.Context, *xmain.State) error {
			return d2rsCLIProcessError("err: failed to fmt: bad usage: no files\nerr: Run with --help to see usage.\n", processErr)
		},
	}
	usage.Start(t, ctx)
	defer usage.Cleanup(t)
	if err := usage.Wait(ctx); err == nil || err.Error() != "failed to wait xmain test: test-d2: failed to fmt: bad usage: no files" {
		t.Fatalf("usage trailer transport changed: %v", err)
	}
	const multiline = "bad usage: format\n\nerr: authored prefix\n  indented\ntrailing  "
	decorated := "err: bad usage: format\nerr:\nerr: err: authored prefix\nerr:   indented\nerr: trailing  \n"
	tms = &xmain.TestState{
		Args: []string{"test-d2"},
		Run: func(context.Context, *xmain.State) error {
			return d2rsCLIProcessError(decorated, processErr)
		},
	}
	tms.Start(t, ctx)
	defer tms.Cleanup(t)
	if err := tms.Wait(ctx); err == nil || err.Error() != "failed to wait xmain test: test-d2: "+multiline {
		t.Fatalf("multiline diagnostic transport changed: %v", err)
	}
}

func (w *d2rsCaptureWriter) Write(p []byte) (int, error) {
	if len(p) > 128*1024*1024-w.captured.Len() {
		return 0, fmt.Errorf("stdout capture exceeds 128 MiB")
	}
	n, err := w.WriteCloser.Write(p)
	w.captured.Write(p[:n])
	return n, err
}

func d2rsCLIRun(original func(context.Context, *xmain.State) error) func(context.Context, *xmain.State) error {
	return func(ctx context.Context, ms *xmain.State) error {
		root := os.Getenv("D2RS_CLI_CAPTURE")
		if root == "" {
			return original(ctx, ms)
		}
		writer := &d2rsCaptureWriter{WriteCloser: ms.Stdout}
		ms.Stdout = writer
		var err error
		if binary := os.Getenv("D2RS_CLI_BINARY"); binary != "" {
			args := append([]string(nil), ms.Opts.Args...)
			// d2cli reads custom fonts with os.ReadFile, while its input/output
			// paths use ms.PWD. Preserve that in-process test distinction when
			// the Rust subprocess uses ms.PWD as its actual working directory.
			for i, arg := range args {
				flag, value, inline := strings.Cut(arg, "=")
				switch flag {
				case "--font-regular", "--font-italic", "--font-bold", "--font-semibold":
					if !inline && i+1 < len(args) {
						value = args[i+1]
					}
					if value != "" {
						absolute, e := filepath.Abs(value)
						if e != nil {
							return e
						}
						if inline {
							args[i] = flag + "=" + absolute
						} else {
							args[i+1] = absolute
						}
					}
				}
			}
			command := exec.CommandContext(ctx, binary, args...)
			command.Dir = ms.PWD
			command.Env = append(os.Environ(), ms.Env.Environ()...)
			transport, transportErr := os.MkdirTemp("", "d2rs-cli-transport-")
			if transportErr != nil {
				return transportErr
			}
			defer os.RemoveAll(transport)
			errorPath := filepath.Join(transport, "error.json")
			logPath := filepath.Join(transport, "process.log")
			if err := os.WriteFile(logPath, nil, 0600); err != nil {
				return err
			}
			command.Env = append(command.Env, "D2_RS_CLI_ERROR_JSON="+errorPath, "D2_RS_CLI_PROCESS_LOG="+logPath)
			command.Stdin = ms.Stdin
			command.Stdout = ms.Stdout
			var stderr bytes.Buffer
			command.Stderr = &stderr
			watch := false
			for _, arg := range args {
				if arg == "--watch" || arg == "-w" || strings.HasPrefix(arg, "--watch=") {
					watch = true
				}
			}
			if watch {
				command.Stderr = io.MultiWriter(ms.Stderr, &stderr)
			}
			err = command.Run()
			processLog, readErr := os.ReadFile(logPath)
			if readErr != nil {
				return readErr
			}
			if _, writeErr := os.Stdout.Write(processLog); writeErr != nil {
				return writeErr
			}
			if ctx.Err() != nil {
				return ctx.Err()
			}
			if err != nil {
				record, readErr := os.ReadFile(errorPath)
				if readErr != nil && !os.IsNotExist(readErr) {
					return readErr
				}
				err = d2rsCLIRecordedError(stderr.String(), err, record)
			} else if !watch {
				if _, e := ms.Stderr.Write(stderr.Bytes()); e != nil {
					return e
				}
			}
		} else {
			err = original(ctx, ms)
		}
		// Stdout is outside the case directory, so preserve it separately.
		if writer.captured.Len() > 0 {
			value, _ := d2rsCLICalls.LoadOrStore(ms.PWD, 0)
			index := value.(int)
			d2rsCLICalls.Store(ms.PWD, index+1)
			dir := filepath.Join(root, "stdout", filepath.Base(ms.PWD))
			if e := os.MkdirAll(dir, 0755); e != nil {
				return e
			}
			if e := os.WriteFile(filepath.Join(dir, fmt.Sprintf("%03d", index)), writer.captured.Bytes(), 0644); e != nil {
				return e
			}
		}
		return err
	}
}

func d2rsCLICapture(t *testing.T, dir string) {
	root := os.Getenv("D2RS_CLI_CAPTURE")
	if root == "" {
		return
	}
	destination := filepath.Join(root, "cases", t.Name())
	if err := os.MkdirAll(destination, 0755); err != nil {
		t.Fatal(err)
	}
	emptyInputs := map[string]bool{}
	d2rsCLIEmptyInputs.Range(func(inputDir, files interface{}) bool {
		relative, err := filepath.Rel(dir, inputDir.(string))
		if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
			return true
		}
		files.(*sync.Map).Range(func(key, value interface{}) bool {
			if value.(bool) {
				emptyInputs[filepath.ToSlash(filepath.Join(relative, key.(string)))] = true
			}
			return true
		})
		return true
	})
	inputBytes, err := json.Marshal(emptyInputs)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(destination, "empty-inputs.json"), inputBytes, 0644); err != nil {
		t.Error(err)
	}
	copyFile := func(source, target string) error {
		info, err := os.Stat(source)
		if err != nil {
			return err
		}
		if info.Size() > 128*1024*1024 {
			return fmt.Errorf("capture file exceeds 128 MiB: %s", source)
		}
		data, err := os.ReadFile(source)
		if err != nil {
			return err
		}
		if len(data) > 128*1024*1024 {
			return fmt.Errorf("capture file exceeds 128 MiB: %s", source)
		}
		if err := os.MkdirAll(filepath.Dir(target), 0755); err != nil {
			return err
		}
		return os.WriteFile(target, data, 0644)
	}
	err = filepath.WalkDir(dir, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.Type()&os.ModeSymlink != 0 {
			return fmt.Errorf("unexpected capture symlink: %s", path)
		}
		if entry.IsDir() {
			return nil
		}
		relative, err := filepath.Rel(dir, path)
		if err != nil {
			return err
		}
		return copyFile(path, filepath.Join(destination, "files", relative))
	})
	if err != nil {
		t.Error(err)
	}
	stdout := filepath.Join(root, "stdout", filepath.Base(dir))
	if _, err := os.Stat(stdout); err == nil {
		if err := os.Rename(stdout, filepath.Join(destination, "stdout")); err != nil {
			t.Error(err)
		}
	}
}

func TestD2rsCLIRecordedError(t *testing.T) {
	processErr := exec.Command("sh", "-c", "exit 7").Run()
	const terminal = "an exit message"
	const api = "an outer context: exiting with code 7: an exit message"
	record := []byte(`{"kind":"exit","code":7,"message":"an exit message","error":"` + api + `"}`)
	if err := d2rsCLIRecordedError("err: "+terminal+"\n", processErr, record); err.Error() != api {
		t.Fatalf("API error context lost: %v", err)
	}
	for _, bad := range [][]byte{[]byte("{"), bytes.ReplaceAll(record, []byte(`"code":7`), []byte(`"code":1`)), bytes.ReplaceAll(record, []byte(`"message":"an exit message"`), []byte(`"message":"wrong"`))} {
		if err := d2rsCLIRecordedError("err: "+terminal+"\n", processErr, bad); err.Error() == api {
			t.Fatal("invalid record accepted")
		}
	}
}
