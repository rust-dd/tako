# File Stream Example

A simple demonstration of Tako's `FileStream` for serving files over HTTP.

## Quick Start

1. **Create a test file:**
   ```bash
   echo "Hello, Tako FileStream!" > test.txt
   ```

2. **Run the example:**
   ```bash
   cargo run
   ```

3. **Test the endpoints:**
   ```bash
   # Basic file serving
   curl http://127.0.0.1:8080/file

   # Range request (first 10 bytes)
   curl -H "Range: bytes=0-10" http://127.0.0.1:8080/video
   ```

## Endpoints

- `GET /file` - Serves the test.txt file
- `GET /video` - Serves the file with range request support

## What it demonstrates

- Basic file streaming without loading entire file into memory
- HTTP range requests for partial content (useful for video streaming)
- Automatic file metadata detection (size, filename)
- Proper error handling for missing files

That's it! A simple example of efficient file serving with Tako.