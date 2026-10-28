#!/bin/sh
# Usage: tla/run-tlc.sh Model.tla  (set TLA2TOOLS to the jar path)
set -eu
: "${TLA2TOOLS:?set TLA2TOOLS to the path of tla2tools.jar}"
exec java -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC -config "${1%.tla}.cfg" "$1"
