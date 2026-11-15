#!/bin/sh
# Test double of the Piper CLI (`piper --model <voice> --output_file <wav>`):
# stores the text read on stdin next to the "voice" and copies the "voice"
# (a WAV sample written by the test) to the output.
cat > "$2.txt"
cp "$2" "$4"
