---
slug: s3-api
title: The S3-compatible API
components: [s3, core, fs]
order: 5
defaultSnippetLines: 24
---

Point `aws s3` or `rclone` at NASDrive and it works: buckets are storage roots,
objects are files, and requests are authenticated with AWS
[Signature Version 4](glossary:sigv4).

This tour follows a request from route registration through signature
verification, credential lookup, bucket-to-root mapping, and finally into
`ListObjectsV2`, `PutObject`, and multipart uploads.

The shape of the implementation is dictated by two facts about the S3 protocol:

- **The URL is not the operation.** S3 overloads a handful of paths with dozens
  of operations selected by query string, so the route table has only six
  entries and the handlers dispatch on `?uploadId`, `?uploads`, and friends
  themselves.
- **The signature covers a reconstruction of the request.** Verification means
  rebuilding the exact canonical string the client hashed — path encoding, query
  sorting, header trimming and all — and comparing in constant time. Most
  compatibility bugs in an S3 implementation live in that reconstruction, so the
  tour spends time there.

The tour is honest about the edges: streaming chunked payloads are accepted but
their framing is not stripped, and multipart ETags do not carry S3's
`-{partcount}` suffix. Those are called out where they live.
