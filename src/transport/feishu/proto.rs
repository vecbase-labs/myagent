/// Feishu WebSocket protobuf frame types (pbbp2 protocol).
/// Manually defined to match the Lark SDK's pbbp2.proto schema.

/// A key-value header in a Frame.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Header {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// A WebSocket frame in the Feishu protocol.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Frame {
    #[prost(int32, tag = "1")]
    pub seq_id: i32,
    #[prost(int32, tag = "2")]
    pub log_id: i32,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, tag = "6")]
    pub payload_encoding: String,
    #[prost(string, tag = "7")]
    pub payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    pub payload: Vec<u8>,
    #[prost(string, tag = "9")]
    pub log_id_new: String,
}

/// Frame method types.
pub const METHOD_CONTROL: i32 = 0;
pub const METHOD_DATA: i32 = 1;

/// Header key constants.
pub const HEADER_TYPE: &str = "type";
pub const HEADER_MESSAGE_ID: &str = "message_id";
pub const HEADER_SUM: &str = "sum";
pub const HEADER_SEQ: &str = "seq";
pub const HEADER_TRACE_ID: &str = "trace_id";
pub const HEADER_BIZ_RT: &str = "biz_rt";

/// Message type constants.
pub const MSG_TYPE_EVENT: &str = "event";
pub const MSG_TYPE_PING: &str = "ping";
pub const MSG_TYPE_PONG: &str = "pong";
