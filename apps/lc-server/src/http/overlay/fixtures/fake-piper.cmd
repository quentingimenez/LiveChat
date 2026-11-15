@echo off
rem Test double of the Piper CLI (piper --model ^<voice^> --output_file ^<wav^>):
rem stores the text read on stdin next to the "voice" and copies the "voice"
rem (a WAV sample written by the test) to the output.
findstr "^" > "%~2.txt"
copy /Y "%~2" "%~4" > nul
