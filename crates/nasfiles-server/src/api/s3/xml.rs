/// Build an S3 XML error response body.
pub fn error_xml(code: &str, message: &str) -> String {
    let escaped = xml_escape(message);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Error>\
           <Code>{code}</Code>\
           <Message>{escaped}</Message>\
         </Error>"
    )
}

/// Escape text for XML: `<`, `>`, `&`, `"`, `'`.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Build a ListBuckets XML response.
pub fn list_buckets_xml(owner_id: &str, owner_name: &str, buckets: &[(String, i64)]) -> String {
    let bucket_items: String = buckets
        .iter()
        .map(|(name, created_ms)| {
            let created = ms_to_iso8601(*created_ms);
            format!(
                "<Bucket><Name>{}</Name><CreationDate>{created}</CreationDate></Bucket>",
                xml_escape(name)
            )
        })
        .collect();

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
           <Owner><ID>{}</ID><DisplayName>{}</DisplayName></Owner>\
           <Buckets>{bucket_items}</Buckets>\
         </ListAllMyBucketsResult>",
        xml_escape(owner_id),
        xml_escape(owner_name),
    )
}

/// Build a ListObjectsV2 XML response.
pub struct ListObjectsV2Result {
    pub bucket: String,
    pub prefix: String,
    pub delimiter: Option<String>,
    pub max_keys: u32,
    pub is_truncated: bool,
    pub key_count: u32,
    pub objects: Vec<S3Object>,
    pub common_prefixes: Vec<String>,
}

pub struct S3Object {
    pub key: String,
    pub size: u64,
    pub last_modified: i64,
    pub etag: String,
}

pub fn list_objects_v2_xml(result: &ListObjectsV2Result) -> String {
    let objects_xml: String = result
        .objects
        .iter()
        .map(|o| {
            let modified = ms_to_iso8601(o.last_modified);
            format!(
                "<Contents>\
                   <Key>{}</Key>\
                   <Size>{}</Size>\
                   <LastModified>{modified}</LastModified>\
                   <ETag>&quot;{}&quot;</ETag>\
                   <StorageClass>STANDARD</StorageClass>\
                 </Contents>",
                xml_escape(&o.key),
                o.size,
                xml_escape(&o.etag),
            )
        })
        .collect();

    let prefixes_xml: String = result
        .common_prefixes
        .iter()
        .map(|p| {
            format!(
                "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
                xml_escape(p)
            )
        })
        .collect();

    let truncated = if result.is_truncated { "true" } else { "false" };
    let delimiter_xml = result
        .delimiter
        .as_deref()
        .map(|d| format!("<Delimiter>{}</Delimiter>", xml_escape(d)))
        .unwrap_or_default();

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
           <Name>{}</Name>\
           <Prefix>{}</Prefix>\
           {delimiter_xml}\
           <MaxKeys>{}</MaxKeys>\
           <IsTruncated>{truncated}</IsTruncated>\
           <KeyCount>{}</KeyCount>\
           {objects_xml}{prefixes_xml}\
         </ListBucketResult>",
        xml_escape(&result.bucket),
        xml_escape(&result.prefix),
        result.max_keys,
        result.key_count,
    )
}

pub fn create_multipart_upload_xml(bucket: &str, key: &str, upload_id: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
           <Bucket>{}</Bucket>\
           <Key>{}</Key>\
           <UploadId>{}</UploadId>\
         </InitiateMultipartUploadResult>",
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(upload_id),
    )
}

pub fn complete_multipart_upload_xml(
    location: &str,
    bucket: &str,
    key: &str,
    etag: &str,
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
           <Location>{}</Location>\
           <Bucket>{}</Bucket>\
           <Key>{}</Key>\
           <ETag>&quot;{}&quot;</ETag>\
         </CompleteMultipartUploadResult>",
        xml_escape(location),
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(etag),
    )
}

pub struct PartInfo {
    pub part_number: u32,
    pub size: u64,
    pub etag: String,
}

pub fn list_parts_xml(bucket: &str, key: &str, upload_id: &str, parts: &[PartInfo]) -> String {
    let parts_xml: String = parts
        .iter()
        .map(|p| {
            format!(
                "<Part>\
                   <PartNumber>{}</PartNumber>\
                   <Size>{}</Size>\
                   <ETag>&quot;{}&quot;</ETag>\
                 </Part>",
                p.part_number,
                p.size,
                xml_escape(&p.etag),
            )
        })
        .collect();

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ListPartsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
           <Bucket>{}</Bucket>\
           <Key>{}</Key>\
           <UploadId>{}</UploadId>\
           {parts_xml}\
         </ListPartsResult>",
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(upload_id),
    )
}

fn ms_to_iso8601(ms: i64) -> String {
    let secs = ms / 1000;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S.000Z").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string())
}
