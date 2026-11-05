// Command otlp-send sends one uniquely named record for each OTLP signal using
// the Collector pdata gRPC clients. It gives native smoke runs real protobuf
// ingestion coverage without relying on the HTTP JSON fixtures.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"time"

	"go.opentelemetry.io/collector/pdata/pcommon"
	"go.opentelemetry.io/collector/pdata/plog"
	"go.opentelemetry.io/collector/pdata/plog/plogotlp"
	"go.opentelemetry.io/collector/pdata/pmetric"
	"go.opentelemetry.io/collector/pdata/pmetric/pmetricotlp"
	"go.opentelemetry.io/collector/pdata/ptrace"
	"go.opentelemetry.io/collector/pdata/ptrace/ptraceotlp"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

func main() {
	endpoint := flag.String("endpoint", "127.0.0.1:4317", "OTLP gRPC receiver address")
	service := flag.String("service", "native-smoke-grpc", "service.name attached to all records")
	flag.Parse()

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	conn, err := grpc.NewClient(*endpoint, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		log.Fatal(err)
	}
	defer conn.Close()

	now := pcommon.NewTimestampFromTime(time.Now().UTC())
	if err := sendTrace(ctx, conn, *service, now); err != nil {
		log.Fatalf("send trace: %v", err)
	}
	if err := sendLog(ctx, conn, *service, now); err != nil {
		log.Fatalf("send log: %v", err)
	}
	if err := sendMetric(ctx, conn, *service, now); err != nil {
		log.Fatalf("send metric: %v", err)
	}
	fmt.Printf("sent one OTLP trace, log, and metric over gRPC to %s (service.name=%s)\n", *endpoint, *service)
}

func addService(resource pcommon.Resource, service string) {
	resource.Attributes().PutStr("service.name", service)
	resource.Attributes().PutStr("native.smoke.transport", "grpc")
}

func sendTrace(ctx context.Context, conn *grpc.ClientConn, service string, now pcommon.Timestamp) error {
	traces := ptrace.NewTraces()
	rs := traces.ResourceSpans().AppendEmpty()
	addService(rs.Resource(), service)
	span := rs.ScopeSpans().AppendEmpty().Spans().AppendEmpty()
	span.SetName("native-smoke-grpc-span")
	span.SetKind(ptrace.SpanKindServer)
	span.SetTraceID(pcommon.TraceID{0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01})
	span.SetSpanID(pcommon.SpanID{0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01})
	span.SetStartTimestamp(now - pcommon.Timestamp(1_000_000_000))
	span.SetEndTimestamp(now)
	span.Attributes().PutStr("native.smoke", "grpc-trace")
	_, err := ptraceotlp.NewGRPCClient(conn).Export(ctx, ptraceotlp.NewExportRequestFromTraces(traces))
	return err
}

func sendLog(ctx context.Context, conn *grpc.ClientConn, service string, now pcommon.Timestamp) error {
	logs := plog.NewLogs()
	rl := logs.ResourceLogs().AppendEmpty()
	addService(rl.Resource(), service)
	record := rl.ScopeLogs().AppendEmpty().LogRecords().AppendEmpty()
	record.SetTimestamp(now)
	record.SetObservedTimestamp(now)
	record.SetSeverityNumber(plog.SeverityNumberInfo)
	record.SetSeverityText("INFO")
	record.Body().SetStr("native-smoke-grpc-log")
	record.Attributes().PutStr("native.smoke", "grpc-log")
	_, err := plogotlp.NewGRPCClient(conn).Export(ctx, plogotlp.NewExportRequestFromLogs(logs))
	return err
}

func sendMetric(ctx context.Context, conn *grpc.ClientConn, service string, now pcommon.Timestamp) error {
	metrics := pmetric.NewMetrics()
	rm := metrics.ResourceMetrics().AppendEmpty()
	addService(rm.Resource(), service)
	metric := rm.ScopeMetrics().AppendEmpty().Metrics().AppendEmpty()
	metric.SetName("native.smoke.grpc.value")
	metric.SetUnit("1")
	metric.SetEmptyGauge()
	point := metric.Gauge().DataPoints().AppendEmpty()
	point.SetStartTimestamp(now - pcommon.Timestamp(1_000_000_000))
	point.SetTimestamp(now)
	point.SetDoubleValue(42.5)
	point.Attributes().PutStr("native.smoke", "grpc-metric")
	_, err := pmetricotlp.NewGRPCClient(conn).Export(ctx, pmetricotlp.NewExportRequestFromMetrics(metrics))
	return err
}
