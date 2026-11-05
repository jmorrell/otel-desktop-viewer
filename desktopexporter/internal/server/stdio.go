package server

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"mime"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"unicode/utf8"

	"golang.org/x/exp/jsonrpc2"
)

const maxStdioFrame = 64 << 20
const maxStdioHeader = 8192

// ServeStdio processes length-delimited requests until the parent closes stdin.
// All writes are serialized; each request receives an independently cancellable context.
func (s *Server) ServeStdio(parent context.Context, input io.Reader, output io.Writer, ready <-chan struct{}, grpcPort, httpPort int) error {
	ctx, cancel := context.WithCancel(parent)
	var workers sync.WaitGroup
	var writeMu, pendingMu sync.Mutex
	pending := make(map[string]context.CancelFunc)
	defer func() { cancel(); workers.Wait() }()
	send := func(id jsonrpc2.ID, result any, rpcErr error) {
		response, err := jsonrpc2.NewResponse(id, result, rpcErr)
		if err != nil {
			return
		}
		body, err := jsonrpc2.EncodeMessage(response)
		if err != nil {
			return
		}
		writeMu.Lock()
		defer writeMu.Unlock()
		if _, err = fmt.Fprintf(output, "Content-Length: %d\r\n\r\n", len(body)); err == nil {
			_, err = output.Write(body)
		}
		if err != nil {
			cancel()
		}
	}
	type frame struct {
		body []byte
		err  error
	}
	frames := make(chan frame)
	go func() {
		reader := bufio.NewReader(input)
		for {
			body, err := readStdioFrame(reader)
			select {
			case frames <- frame{body, err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	for {
		var next frame
		select {
		case <-ctx.Done():
			return nil
		case next = <-frames:
		}
		body, err := next.body, next.err
		if err != nil {
			if err == io.EOF {
				return nil
			}
			return err
		}
		if !utf8.Valid(body) {
			send(jsonrpc2.ID{}, nil, jsonrpc2.ErrParse)
			continue
		}
		var envelope struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
			Params json.RawMessage `json:"params"`
		}
		if err := json.Unmarshal(body, &envelope); err != nil {
			send(jsonrpc2.ID{}, nil, jsonrpc2.ErrParse)
			continue
		}
		msg, err := jsonrpc2.DecodeMessage(body)
		if err != nil {
			send(jsonrpc2.ID{}, nil, jsonrpc2.ErrInvalidRequest)
			continue
		}
		req, ok := msg.(*jsonrpc2.Request)
		if !ok {
			send(jsonrpc2.ID{}, nil, jsonrpc2.ErrInvalidRequest)
			continue
		}
		if envelope.Method == "$/cancelRequest" {
			var params struct {
				ID json.RawMessage `json:"id"`
			}
			if json.Unmarshal(envelope.Params, &params) == nil {
				pendingMu.Lock()
				stop := pending[string(params.ID)]
				pendingMu.Unlock()
				if stop != nil {
					stop()
				}
			}
			continue
		}
		if len(envelope.ID) == 0 {
			continue
		}
		key := string(envelope.ID)
		requestCtx, stop := context.WithCancel(ctx)
		pendingMu.Lock()
		if _, exists := pending[key]; exists {
			pendingMu.Unlock()
			stop()
			send(req.ID, nil, jsonrpc2.ErrInvalidRequest)
			continue
		}
		pending[key] = stop
		pendingMu.Unlock()
		workers.Add(1)
		go func() {
			defer workers.Done()
			defer stop()
			defer func() { pendingMu.Lock(); delete(pending, key); pendingMu.Unlock() }()
			var result any
			var rpcErr error
			switch req.Method {
			case "native.initialize":
				select {
				case <-ready:
					result = map[string]any{"version": 1, "capabilities": []string{"export", "cancellation"}, "receiverPorts": map[string]int{"grpc": grpcPort, "http": httpPort}}
				case <-requestCtx.Done():
					rpcErr = mapStoreError(requestCtx.Err())
				}
			case "native.export":
				result, rpcErr = s.nativeExport(requestCtx, envelope.Params)
			default:
				result, rpcErr = s.jsonrpcHandler.Handle(requestCtx, req)
			}
			send(req.ID, result, rpcErr)
		}()
	}
}

func readStdioFrame(r *bufio.Reader) ([]byte, error) {
	length := -1
	headerBytes := 0
	for {
		line, err := r.ReadSlice('\n')
		headerBytes += len(line)
		if headerBytes > maxStdioHeader || err == bufio.ErrBufferFull {
			return nil, fmt.Errorf("stdio header too large")
		}
		if err != nil {
			if err == io.EOF && headerBytes == 0 {
				return nil, io.EOF
			}
			return nil, fmt.Errorf("incomplete stdio header: %w", err)
		}
		if !bytes.HasSuffix(line, []byte("\r\n")) {
			return nil, fmt.Errorf("invalid stdio header line")
		}
		text := string(line[:len(line)-2])
		if text == "" {
			break
		}
		key, value, ok := strings.Cut(text, ":")
		if !ok {
			return nil, fmt.Errorf("invalid stdio header")
		}
		if strings.EqualFold(key, "Content-Length") {
			if length != -1 {
				return nil, fmt.Errorf("duplicate content length")
			}
			length, err = strconv.Atoi(strings.TrimSpace(value))
			if err != nil || length < 0 || length > maxStdioFrame {
				return nil, fmt.Errorf("invalid content length")
			}
		}
	}
	if length < 0 {
		return nil, fmt.Errorf("missing content length")
	}
	body := make([]byte, length)
	if _, err := io.ReadFull(r, body); err != nil {
		return nil, err
	}
	return body, nil
}

// nativeExport routes through the HTTP export handler so validation and bytes stay identical.
func (s *Server) nativeExport(ctx context.Context, raw json.RawMessage) (any, error) {
	var p struct {
		Signal            string            `json:"signal"`
		Reference         string            `json:"reference"`
		Scope             map[string]string `json:"scope"`
		IncludeDatapoints *bool             `json:"includeDatapoints"`
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if dec.Decode(&p) != nil || p.Signal == "" || p.Reference == "" {
		return nil, jsonrpc2.ErrInvalidParams
	}
	if p.Signal != "traces" && p.Signal != "logs" && p.Signal != "metrics" {
		return nil, jsonrpc2.ErrInvalidParams
	}
	values := url.Values{}
	for key, value := range p.Scope {
		values.Set(key, value)
	}
	if p.IncludeDatapoints != nil {
		values.Set("includeDatapoints", strconv.FormatBool(*p.IncludeDatapoints))
	}
	req := httptest.NewRequest("GET", "http://localhost/export/"+url.PathEscape(p.Signal)+"/"+url.PathEscape(p.Reference)+"?"+values.Encode(), nil).WithContext(ctx)
	req.SetPathValue("signal", p.Signal)
	req.SetPathValue("id", p.Reference)
	recorder := httptest.NewRecorder()
	s.exportHandler(recorder, req)
	if recorder.Code != 200 {
		if ctx.Err() != nil {
			return nil, mapStoreError(ctx.Err())
		}
		if recorder.Code == 400 {
			return nil, jsonrpc2.ErrInvalidParams
		}
		if recorder.Code == 404 {
			switch p.Signal {
			case "traces":
				return nil, ErrTraceNotFound
			case "logs":
				return nil, ErrLogsNotFound
			case "metrics":
				return nil, ErrMetricNotFound
			}
		}
		return nil, jsonrpc2.ErrInternal
	}
	_, params, err := mime.ParseMediaType(recorder.Header().Get("Content-Disposition"))
	if err != nil {
		return nil, jsonrpc2.ErrInternal
	}
	return map[string]string{"filename": params["filename"], "content": recorder.Body.String()}, nil
}
