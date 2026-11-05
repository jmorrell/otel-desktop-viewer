package server

import (
	"bufio"
	"bytes"
	"context"
	"fmt"
	"io"
	"strings"
	"testing"
)

func TestStdioFrames(t *testing.T) {
	body := `{"jsonrpc":"2.0","id":1,"method":"getStats"}`
	wire := fmt.Sprintf("Content-Length: %d\r\n\r\n%s", len(body), body)
	// A one-byte reader exercises headers and bodies arriving in fragments.
	got, err := readStdioFrame(bufio.NewReader(&singleByteReader{r: strings.NewReader(wire)}))
	if err != nil || string(got) != body {
		t.Fatalf("got %q, %v", got, err)
	}
	for _, input := range []string{"Content-Length: 999999999\r\n\r\n", "Content-Length: -1\r\n\r\n", "Content-Length: 1\r\nContent-Length: 1\r\n\r\nx", "Content-Length: 3\r\n\r\nx", "Content-Length: 1\n\nx", strings.Repeat("x", maxStdioHeader+1)} {
		if _, err := readStdioFrame(bufio.NewReader(strings.NewReader(input))); err == nil {
			t.Errorf("accepted invalid frame %q", input[:min(len(input), 40)])
		}
	}
	if _, err := readStdioFrame(bufio.NewReader(bytes.NewReader(nil))); err != io.EOF {
		t.Fatalf("EOF: %v", err)
	}
}

type singleByteReader struct{ r io.Reader }

func (s *singleByteReader) Read(p []byte) (int, error) { return s.r.Read(p[:1]) }

func TestStdioConcurrentAndCancellation(t *testing.T) {
	inReader, inWriter := io.Pipe()
	outReader, outWriter := io.Pipe()
	ready := make(chan struct{})
	done := make(chan error, 1)
	go func() {
		done <- (&Server{}).ServeStdio(context.Background(), inReader, outWriter, ready, 4317, 4318)
		outWriter.Close()
	}()
	send := func(body string) {
		t.Helper()
		if _, err := fmt.Fprintf(inWriter, "Content-Length: %d\r\n\r\n%s", len(body), body); err != nil {
			t.Fatal(err)
		}
	}
	send(`{"jsonrpc":"2.0","id":1,"method":"native.initialize"}`)
	send(`{"jsonrpc":"2.0","id":2,"method":"native.initialize"}`)
	send(`{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":1}}`)
	reader := bufio.NewReader(outReader)
	first, err := readStdioFrame(reader)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(first, []byte(`"id":1`)) || !bytes.Contains(first, []byte(`-32010`)) {
		t.Fatalf("cancellation: %s", first)
	}
	close(ready)
	second, err := readStdioFrame(reader)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(second, []byte(`"id":2`)) || !bytes.Contains(second, []byte(`"grpc":4317`)) {
		t.Fatalf("initialize: %s", second)
	}
	inWriter.Close()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}

func TestStdioShutdownInterruptsBlockedInputAndInitialization(t *testing.T) {
	input, writer := io.Pipe()
	defer writer.Close()
	output, outputWriter := io.Pipe()
	defer output.Close()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		done <- (&Server{}).ServeStdio(ctx, input, outputWriter, make(chan struct{}), 4317, 4318)
		outputWriter.Close()
	}()
	body := `{"jsonrpc":"2.0","id":1,"method":"native.initialize"}`
	if _, err := fmt.Fprintf(writer, "Content-Length: %d\r\n\r\n%s", len(body), body); err != nil {
		t.Fatal(err)
	}
	cancel()
	response, err := io.ReadAll(output)
	if err != nil {
		t.Fatal(err)
	}
	if len(response) > 0 && !bytes.Contains(response, []byte(`-32010`)) {
		t.Fatalf("response: %s", response)
	}
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}
