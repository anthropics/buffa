package bench_test

import (
	"os"
	"testing"

	benchpb "github.com/anthropics/buffa/benchmarks/go/gen/bench"
	benchmarkspb "github.com/anthropics/buffa/benchmarks/go/gen/benchmarks"
	proto3pb "github.com/anthropics/buffa/benchmarks/go/gen/proto3"
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/proto"
)

// loadDataset reads a .pb file and returns the decoded BenchmarkDataset.
func loadDataset(tb testing.TB, path string) *benchmarkspb.BenchmarkDataset {
	tb.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		tb.Fatalf("failed to read dataset %s: %v", path, err)
	}
	ds := &benchmarkspb.BenchmarkDataset{}
	if err := proto.Unmarshal(data, ds); err != nil {
		tb.Fatalf("failed to unmarshal dataset %s: %v", path, err)
	}
	if len(ds.Payload) == 0 {
		tb.Fatalf("dataset %s has no payloads", path)
	}
	return ds
}

// decodePayloads unmarshals all binary payloads in a dataset to proto messages.
func decodePayloads(tb testing.TB, ds *benchmarkspb.BenchmarkDataset, newMsg func() proto.Message) []proto.Message {
	tb.Helper()
	msgs := make([]proto.Message, len(ds.Payload))
	for i, payload := range ds.Payload {
		msg := newMsg()
		if err := proto.Unmarshal(payload, msg); err != nil {
			tb.Fatalf("failed to unmarshal payload %d: %v", i, err)
		}
		msgs[i] = msg
	}
	return msgs
}

// prepareJsonData encodes all messages to JSON and returns the byte slices
// plus total byte count.
func prepareJsonData(tb testing.TB, msgs []proto.Message) ([][]byte, int64) {
	tb.Helper()
	jsonData := make([][]byte, len(msgs))
	var total int64
	for i, msg := range msgs {
		data, err := protojson.Marshal(msg)
		if err != nil {
			tb.Fatalf("failed to marshal message %d to JSON: %v", i, err)
		}
		jsonData[i] = data
		total += int64(len(data))
	}
	return jsonData, total
}

// benchJsonEncode benchmarks protojson.Marshal for a set of pre-decoded messages.
func benchJsonEncode(b *testing.B, msgs []proto.Message, totalJsonBytes int64) {
	b.SetBytes(totalJsonBytes)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, msg := range msgs {
			out, err := protojson.Marshal(msg)
			if err != nil {
				b.Fatal(err)
			}
			_ = out
		}
	}
}

// benchJsonDecode benchmarks protojson.Unmarshal for a set of pre-encoded JSON strings.
func benchJsonDecode(b *testing.B, jsonData [][]byte, newMsg func() proto.Message, totalJsonBytes int64) {
	b.SetBytes(totalJsonBytes)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, data := range jsonData {
			msg := newMsg()
			if err := protojson.Unmarshal(data, msg); err != nil {
				b.Fatal(err)
			}
		}
	}
}

type benchCase struct {
	name    string
	dataset string
	newMsg  func() proto.Message
}

var cases = []benchCase{
	{
		name:    "ApiResponse",
		dataset: "../datasets/api_response.pb",
		newMsg:  func() proto.Message { return &benchpb.ApiResponse{} },
	},
	{
		name:    "LogRecord",
		dataset: "../datasets/log_record.pb",
		newMsg:  func() proto.Message { return &benchpb.LogRecord{} },
	},
	{
		name:    "AnalyticsEvent",
		dataset: "../datasets/analytics_event.pb",
		newMsg:  func() proto.Message { return &benchpb.AnalyticsEvent{} },
	},
	{
		name:    "GoogleMessage1",
		dataset: "../datasets/google_message1_proto3.pb",
		newMsg:  func() proto.Message { return &proto3pb.GoogleMessage1{} },
	},
	{
		name:    "MediaFrame",
		dataset: "../datasets/media_frame.pb",
		newMsg:  func() proto.Message { return &benchpb.MediaFrame{} },
	},
}

// benchBinaryEncode benchmarks proto.Marshal for a set of pre-decoded messages.
func benchBinaryEncode(b *testing.B, msgs []proto.Message, totalBinaryBytes int64) {
	b.SetBytes(totalBinaryBytes)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, msg := range msgs {
			out, err := proto.Marshal(msg)
			if err != nil {
				b.Fatal(err)
			}
			_ = out
		}
	}
}

// benchBinaryDecode benchmarks proto.Unmarshal for a set of binary payloads.
func benchBinaryDecode(b *testing.B, payloads [][]byte, newMsg func() proto.Message, totalBytes int64) {
	b.SetBytes(totalBytes)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, payload := range payloads {
			msg := newMsg()
			if err := proto.Unmarshal(payload, msg); err != nil {
				b.Fatal(err)
			}
			_ = msg
		}
	}
}

func BenchmarkBinaryEncode(b *testing.B) {
	for _, tc := range cases {
		b.Run(tc.name, func(b *testing.B) {
			ds := loadDataset(b, tc.dataset)
			msgs := decodePayloads(b, ds, tc.newMsg)
			var totalBytes int64
			for _, p := range ds.Payload {
				totalBytes += int64(len(p))
			}
			benchBinaryEncode(b, msgs, totalBytes)
		})
	}
}

func BenchmarkBinaryDecode(b *testing.B) {
	for _, tc := range cases {
		b.Run(tc.name, func(b *testing.B) {
			ds := loadDataset(b, tc.dataset)
			var totalBytes int64
			for _, p := range ds.Payload {
				totalBytes += int64(len(p))
			}
			benchBinaryDecode(b, ds.Payload, tc.newMsg, totalBytes)
		})
	}
}

func BenchmarkJsonEncode(b *testing.B) {
	for _, tc := range cases {
		b.Run(tc.name, func(b *testing.B) {
			ds := loadDataset(b, tc.dataset)
			msgs := decodePayloads(b, ds, tc.newMsg)
			_, totalBytes := prepareJsonData(b, msgs)
			benchJsonEncode(b, msgs, totalBytes)
		})
	}
}

func BenchmarkJsonDecode(b *testing.B) {
	for _, tc := range cases {
		b.Run(tc.name, func(b *testing.B) {
			ds := loadDataset(b, tc.dataset)
			msgs := decodePayloads(b, ds, tc.newMsg)
			jsonData, totalBytes := prepareJsonData(b, msgs)
			benchJsonDecode(b, jsonData, tc.newMsg, totalBytes)
		})
	}
}
