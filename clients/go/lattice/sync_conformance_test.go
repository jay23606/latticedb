package lattice

import (
	"reflect"
	"sort"
	"testing"
)

// Every sync vector the Rust side wrote must decode, encode and fail exactly the same way here.

func TestSyncVectorFileShape(t *testing.T) {
	if vectors.Format != "lattice-conformance-vectors" || vectors.Version != 1 {
		t.Fatalf("unexpected vector file %q version %d", vectors.Format, vectors.Version)
	}
}

func TestSyncValidFramesDecodeAndEncodeByteForByte(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "valid") {
		name := str(t, field(t, v, "name"))
		raw := hexBytes(t, field(t, v, "bytes"))
		want := messageFromJSON(t, obj0(t, field(t, v, "message")))

		got, err := Decode(raw)
		if err != nil || got == nil {
			t.Fatalf("%s: decode: %v", name, err)
		}
		if got.Length != len(raw) || !reflect.DeepEqual(got.Message, want) {
			t.Fatalf("%s: decoded %#v (%d bytes), want %#v (%d bytes)", name, got.Message, got.Length, want, len(raw))
		}
		enc, err := Encode(want)
		if err != nil || !reflect.DeepEqual(enc, raw) {
			t.Fatalf("%s: encode = %x, %v; want %x", name, enc, err, raw)
		}
		again, err := Encode(got.Message)
		if err != nil || !reflect.DeepEqual(again, raw) {
			t.Fatalf("%s: encode(decode(bytes)) = %x, %v; want %x", name, again, err, raw)
		}
	}
}

func TestSyncFrameFollowedByMoreBytesDecodesAlone(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "valid") {
		raw := hexBytes(t, field(t, v, "bytes"))
		more := append(append([]byte{}, raw...), 0xff, 0x00, 0x01)
		got, err := Decode(more)
		if err != nil || got == nil {
			t.Fatalf("%s: %v", v["name"], err)
		}
		if got.Length != len(raw) {
			t.Fatalf("%s: consumed %d bytes, want %d", v["name"], got.Length, len(raw))
		}
	}
}

func TestSyncEveryProperPrefixOfAValidFrameIsIncomplete(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "valid") {
		raw := hexBytes(t, field(t, v, "bytes"))
		for cut := range raw {
			got, err := Decode(raw[:cut])
			if err != nil || got != nil {
				t.Fatalf("%s: prefix %d: got %v, %v; want incomplete", v["name"], cut, got, err)
			}
		}
	}
}

func TestSyncInvalidFramesFailWithTheErrorRustReports(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "invalid") {
		name := str(t, field(t, v, "name"))
		limit := uint64(MaxPayload)
		if l, ok := v["limit"]; ok {
			limit = num(t, l)
		}
		got, err := DecodeWithLimit(hexBytes(t, field(t, v, "bytes")), limit)
		if got != nil {
			t.Fatalf("%s: decoded a frame that must be refused", name)
		}
		assertErrorMatches(t, err, obj0(t, field(t, v, "error")), name)
	}
}

func TestSyncIncompleteInputReturnsNothing(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "incomplete") {
		got, err := Decode(hexBytes(t, field(t, v, "bytes")))
		if got != nil || err != nil {
			t.Fatalf("%s: got %v, %v; want incomplete", v["name"], got, err)
		}
	}
}

func TestSyncEncoderRefusesWhatRustRefuses(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "encode_errors") {
		name := str(t, field(t, v, "name"))
		_, err := Encode(messageFromJSON(t, obj0(t, field(t, v, "message"))))
		assertErrorMatches(t, err, obj0(t, field(t, v, "error")), name)
	}
}

func TestSyncVersionNegotiationGivesRustsAnswer(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "negotiate") {
		minimum := uint16(num(t, field(t, v, "min")))
		maximum := uint16(num(t, field(t, v, "max")))
		got := Negotiate(minimum, maximum)
		want := obj0(t, field(t, v, "result"))
		if ver, ok := want["version"]; ok {
			if got.Error != "" || uint64(got.Version) != num(t, ver) {
				t.Fatalf("%d..=%d: got %+v, want version %v", minimum, maximum, got, ver)
			}
		} else if got.Error != str(t, field(t, want, "error")) {
			t.Fatalf("%d..=%d: got %+v, want error %v", minimum, maximum, got, want["error"])
		}
	}
}

func TestSyncVectorsCoverEveryMessageTypeAndErrorKind(t *testing.T) {
	types := map[string]bool{}
	for _, v := range rows(t, vectors.Sync, "valid") {
		types[str(t, field(t, obj0(t, field(t, v, "message")), "type"))] = true
	}
	want := []string{"ack", "error", "events", "hello", "ping", "pong", "subscribe", "welcome"}
	assertSameSet(t, "message types", types, want)

	kinds := map[string]bool{}
	for _, v := range rows(t, vectors.Sync, "invalid") {
		kinds[str(t, field(t, obj0(t, field(t, v, "error")), "kind"))] = true
	}
	assertSameSet(t, "invalid kinds", kinds, []string{
		"bad_length", "invalid_field", "not_utf8", "trailing", "truncated", "unknown_type",
	})
}

func assertSameSet(t *testing.T, what string, got map[string]bool, want []string) {
	t.Helper()
	var names []string
	for k := range got {
		names = append(names, k)
	}
	sort.Strings(names)
	sort.Strings(want)
	if !reflect.DeepEqual(names, want) {
		t.Fatalf("%s: %v, want %v", what, names, want)
	}
}

func TestSyncEventsFrameCarriesAFeedEvent(t *testing.T) {
	for _, v := range rows(t, vectors.Sync, "valid") {
		if v["name"] != "events_carrying_a_feed_event" {
			continue
		}
		got, err := Decode(hexBytes(t, field(t, v, "bytes")))
		if err != nil || got == nil {
			t.Fatalf("decode: %v", err)
		}
		events := got.Message.(Events).Events
		if len(events) == 0 {
			t.Fatal("no events")
		}
		inner, err := DecodeEvent(events[0].Data)
		if err != nil {
			t.Fatalf("inner event: %v", err)
		}
		if !reflect.DeepEqual(CursorToBytes(inner.Cursor), events[0].Cursor) {
			t.Fatalf("inner cursor %x does not match the frame cursor %x", CursorToBytes(inner.Cursor), events[0].Cursor)
		}
		return
	}
	t.Fatal("vector events_carrying_a_feed_event is missing")
}
