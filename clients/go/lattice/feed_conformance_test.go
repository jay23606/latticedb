package lattice

import (
	"reflect"
	"testing"
)

// Every change feed vector the Rust side wrote must decode, encode and fail exactly the same way.

func TestFeedCursorsEncodeAndDecodeAsRustDoesAndByteOrderLikeCursors(t *testing.T) {
	listing := rows(t, vectors.Feed, "cursors")
	for _, v := range listing {
		name := str(t, field(t, v, "name"))
		raw := hexBytes(t, field(t, v, "bytes"))
		c, err := CursorFromBytes(raw)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		cur := obj0(t, field(t, v, "cursor"))
		wantLSN := decimal(t, field(t, cur, "lsn"))
		wantIndex := uint32(num(t, field(t, cur, "index")))
		if c.LSN != wantLSN || c.Index != wantIndex {
			t.Fatalf("%s: decoded %+v, want lsn %d index %d", name, c, wantLSN, wantIndex)
		}
		if !reflect.DeepEqual(CursorToBytes(c), raw) {
			t.Fatalf("%s: encode = %x, want %x", name, CursorToBytes(c), raw)
		}
	}
	for _, a := range listing {
		for _, b := range listing {
			ba := hexBytes(t, field(t, a, "bytes"))
			bb := hexBytes(t, field(t, b, "bytes"))
			ca, _ := CursorFromBytes(ba)
			cb, _ := CursorFromBytes(bb)
			if CompareEncodedCursors(ba, bb) != CompareCursors(ca, cb) {
				t.Fatalf("%s vs %s: byte order %d, cursor order %d", a["name"], b["name"],
					CompareEncodedCursors(ba, bb), CompareCursors(ca, cb))
			}
		}
	}
}

func TestFeedInvalidCursorsFailWithRustsError(t *testing.T) {
	for _, v := range rows(t, vectors.Feed, "cursors_invalid") {
		name := str(t, field(t, v, "name"))
		_, err := CursorFromBytes(hexBytes(t, field(t, v, "bytes")))
		assertErrorMatches(t, err, obj0(t, field(t, v, "error")), name)
	}
}

func TestFeedValidEventsDecodeAndEncodeByteForByte(t *testing.T) {
	for _, v := range rows(t, vectors.Feed, "events") {
		name := str(t, field(t, v, "name"))
		raw := hexBytes(t, field(t, v, "bytes"))
		want := eventFromJSON(t, obj0(t, field(t, v, "event")))

		got, err := DecodeEvent(raw)
		if err != nil {
			t.Fatalf("%s: decode: %v", name, err)
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("%s: decoded %#v, want %#v", name, got, want)
		}
		enc, err := EncodeEvent(want)
		if err != nil || !reflect.DeepEqual(enc, raw) {
			t.Fatalf("%s: encode = %x, %v; want %x", name, enc, err, raw)
		}
		again, err := EncodeEvent(got)
		if err != nil || !reflect.DeepEqual(again, raw) {
			t.Fatalf("%s: round trip = %x, %v; want %x", name, again, err, raw)
		}
	}
}

func TestFeedEveryProperPrefixOfAValidEventIsTruncated(t *testing.T) {
	for _, v := range rows(t, vectors.Feed, "events") {
		raw := hexBytes(t, field(t, v, "bytes"))
		for cut := range raw {
			_, err := DecodeEvent(raw[:cut])
			assertErrorMatches(t, err, obj{"kind": "truncated"}, v["name"].(string))
		}
	}
}

func TestFeedInvalidEventsFailWithRustsError(t *testing.T) {
	for _, v := range rows(t, vectors.Feed, "events_invalid") {
		name := str(t, field(t, v, "name"))
		_, err := DecodeEvent(hexBytes(t, field(t, v, "bytes")))
		assertErrorMatches(t, err, obj0(t, field(t, v, "error")), name)
	}
}

func TestFeedEncoderRefusesWhatRustRefuses(t *testing.T) {
	for _, v := range rows(t, vectors.Feed, "encode_errors") {
		name := str(t, field(t, v, "name"))
		event := eventFromJSON(t, obj0(t, field(t, v, "event")))
		_, err := EncodeEvent(event)
		assertErrorMatches(t, err, obj0(t, field(t, v, "error")), name)
	}
}
