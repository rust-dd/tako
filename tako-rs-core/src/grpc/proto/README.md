The schemas are from [grpc-proto](https://github.com/grpc/grpc-proto). The Rust
message types are checked in alongside the services; consumers do not run protoc.
The descriptor sets let reflection describe itself and the health service.

Regenerate the descriptors from the repository root when changing a schema:

```sh
protoc --proto_path=tako-rs-core/src/grpc/proto \
  --descriptor_set_out=tako-rs-core/src/grpc/proto/reflection.bin \
  grpc/reflection/v1/reflection.proto grpc/reflection/v1alpha/reflection.proto
protoc --proto_path=tako-rs-core/src/grpc/proto \
  --descriptor_set_out=tako-rs-core/src/grpc/proto/health.bin \
  grpc/health/v1/health.proto
```
