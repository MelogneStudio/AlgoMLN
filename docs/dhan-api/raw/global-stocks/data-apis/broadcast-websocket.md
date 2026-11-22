# Global Stocks Live Feed

Real-time Global Stocks market data over WebSocket.

The Global Stocks Live Feed streams real-time market data for supported international stocks. Client applications send JSON subscription requests, while the server sends compact binary market-data packets over WebSocket.

- Protocol: WebSocket, binary frames (opcode `2`)
- Client request format: JSON
- Server response format: binary WebSocket frames
- Byte order: little-endian for all multi-byte numbers
- Supported exchange segment: `INX_EQ`

## Establishing Connection

Open a WebSocket connection to:

```text
wss://<host>/?clientId=<clientId>&token=<authToken>&authType=<type>&version=2
```

For the Dhan-hosted Global Stocks feed, use:

```text
wss://global-stocks-api-feed.dhan.co/?clientId=<your-client-id>&token=<your-access-token>&authType=<type>&version=2
```

Self-hosted deployments typically expose the feed over `ws` on port `7075`.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `clientId` | Yes | Dhan Client ID. For partner authentication, pass the Partner ID. |
| `token` | Yes | Access token. For partner authentication, pass the secret key. |
| `authType` | Yes | `1` for Partner's Client, `2` for Self, `3` for Partner. |
| `version` | Yes | Must be `2`. |

Connection handling:

- On successful authentication, the connection remains open for subscriptions.
- On authentication failure, the server sends an Error packet and closes the connection.
- A maximum of 5 concurrent WebSocket connections are allowed per `clientId`. If a 6th connection is opened, the oldest connection is disconnected with error code `805`.
- More than 10 repeated invalid connection attempts may temporarily block the source IP.
- If the WebSocket upgrade fails, inspect the HTTP status and response body. `EOF` or a missing HTTP response indicates that the server closed the TLS/WebSocket handshake before accepting the connection. `400 Bad Request` indicates that the query parameters were received but rejected. Stop the connection flow and recheck `clientId`, `token`, `authType`, and `version` before sending any request.

## Request Messages

All request messages are JSON. Each request contains a `requestCode`.

| requestCode | Action |
|-------------|--------|
| `12` | Disconnect |
| `15` | Subscribe Trade |
| `16` | Unsubscribe Trade |
| `17` | Subscribe OHLC |
| `18` | Unsubscribe OHLC |

### Subscribe or Unsubscribe

Use this request structure for request codes `15`, `16`, `17`, and `18`.

```json
{
  "requestCode": 15,
  "instrumentCount": 2,
  "instrumentList": [
    { "exchangeSegment": "INX_EQ", "securityId": "1234" },
    { "exchangeSegment": "INX_EQ", "securityId": "5678" }
  ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `requestCode` | int | Request code for subscribing or unsubscribing Trade or OHLC data. |
| `instrumentCount` | int | Number of instruments in `instrumentList`. Maximum 100 instruments per request. |
| `instrumentList` | array | List of instruments to subscribe or unsubscribe. |
| `instrumentList[].exchangeSegment` | string | Currently supported value: `INX_EQ`. |
| `instrumentList[].securityId` | string | Security ID of the instrument. |

Each connection supports up to 5000 total subscriptions. Trade and OHLC subscriptions are maintained independently for each instrument.

To receive both Trade and OHLC for the same instrument, send separate subscribe requests for `requestCode` `15` and `17`.

### Disconnect

```json
{ "requestCode": 12 }
```

## Keeping Connection Alive

The server sends WebSocket Ping frames approximately every 10 seconds. Most WebSocket libraries automatically reply with Pong frames.

If 4 consecutive Ping frames are not answered, the server closes the connection.

If the connection closes abnormally, for example with close `1006` or `unexpected EOF`, reconnect and re-subscribe.

## Binary Response

Binary messages consist of sequences of bytes that represent the data. Binary messages require parsing to extract the relevant information.

All responses from Global Stocks Live Feed consist of [Response Header](#response-header) and Payload. Header for every response message remains the same with different `MsgCode`, while the payload can be different.

The data on Global Stocks Live Feed is sent in little-endian byte order. Structs are packed with no alignment padding.

A single WebSocket message can contain one packet or multiple packets concatenated together. Parse each message in a loop until the full binary buffer is consumed.

### Response Header

The response header message is of 11 bytes and remains part of every response message. The message structure is given below.

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0` | uint8 | 1 | Exchange segment. `14` represents `INX_EQ`. |
| `1-4` | int32 | 4 | Security ID. |
| `5-8` | int32 | 4 | Duplicate of Security ID. |
| `9` | uint8 | 1 | Message length of the entire packet. |
| `10` | uint8 | 1 | Message code. |

`MsgCode` is byte `10` of the header. Reading byte `0` as the message code will read the exchange segment instead.

### Message Codes

| MsgCode | Packet | Total Size |
|---------|--------|------------|
| `1` | Trade | 27 bytes |
| `3` | OHLC | 27 bytes |
| `29` | Market Status | 18 bytes |
| `32` | Previous Close | 15 bytes |
| `33` | Circuit Limit | 19 bytes |
| `36` | 52-Week High/Low | 19 bytes |
| `50` | Error | 13 bytes |

### Trade Packet

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0-10` | [ ] byte | 11 | [Response Header](#response-header) with `MsgCode = 1`. |
| `11-14` | float32 | 4 | LTP - Last Traded Price. |
| `15-18` | int32 | 4 | Volume. |
| `19-22` | int32 | 4 | LTT - Last Trade Time, in Unix epoch seconds for the feed timestamp. |
| `23-26` | int32 | 4 | LUT - Last Update Time, in Unix epoch seconds. |

### OHLC Packet

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0-10` | [ ] byte | 11 | [Response Header](#response-header) with `MsgCode = 3`. |
| `11-14` | float32 | 4 | FOpen - Open. |
| `15-18` | float32 | 4 | FClose - Close. |
| `19-22` | float32 | 4 | FHigh - High. |
| `23-26` | float32 | 4 | FLow - Low. |

### Previous Close Packet

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0-10` | [ ] byte | 11 | [Response Header](#response-header) with `MsgCode = 32`. |
| `11-14` | float32 | 4 | PClose - Previous Close. |

### Circuit Limit Packet

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0-10` | [ ] byte | 11 | [Response Header](#response-header) with `MsgCode = 33`. |
| `11-14` | float32 | 4 | UCkt - Upper Circuit. |
| `15-18` | float32 | 4 | LCkt - Lower Circuit. |

### 52-Week High/Low Packet

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0-10` | [ ] byte | 11 | [Response Header](#response-header) with `MsgCode = 36`. |
| `11-14` | float32 | 4 | F52WKHIGH - 52-Week High. |
| `15-18` | float32 | 4 | F52WKLOW - 52-Week Low. |

### Market Status Packet

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0-10` | [ ] byte | 11 | [Response Header](#response-header) with `MsgCode = 29`. |
| `11-13` | uint8[3] | 3 | MarketType. |
| `14-17` | int32 | 4 | MarketStatus. |

## Feed Disconnect

In case of authentication failure or server-side disconnection, you will receive an Error packet with disconnection reason code. After sending the Error packet, the server closes the connection.

Error packets use `MsgCode = 50`, total length 13 bytes, and are sent as standalone binary messages. Error packets are not batched with market-data packets. Error packets use the same 11-byte header as market-data packets, followed by a 2-byte error code.

| Bytes | Type | Size | Description |
|-------|------|------|-------------|
| `0` | uint8 | 1 | ExchSeg. `0` when not set. |
| `1-4` | int32 | 4 | ScripId. `0` when not set. |
| `5-8` | int32 | 4 | ScripId2. `0` when not set. |
| `9` | uint8 | 1 | MsgLength. `13`. |
| `10` | uint8 | 1 | MsgCode. `50`. |
| `11-12` | int16 | 2 | Disconnection message code. |

| ErrCode | Description |
|---------|-------------|
| `805` | Connection limit exceeded. |
| `806` | Payment not done for broadcast. |
| `807` | Token expired. |
| `808` | Authentication failure. |
| `809` | Invalid token. |

## First Tick Snapshot

Subscription flow:

- Subscribe Trade, `requestCode = 15`: the server sends, per instrument and in order, `Trade(1)`, `PClose(32)`, `CircuitLimit(33)`, and `52WK(36)`, followed by live `Trade(1)` updates.
- Subscribe OHLC, `requestCode = 17`: the server sends an `OHLC(3)` snapshot, followed by live `OHLC(3)` updates.
- `PClose(32)`, `CircuitLimit(33)`, and `52WK(36)` are sent once on subscription. They are not part of every live Trade update.
- `MarketStatus(29)` is pushed on market open, close, and similar session state changes.
- Keep-alive: the server sends Ping frames about every 10 seconds. The client must send Pong frames; 4 missed Pong responses disconnect the connection.

For OHLC and reference packets:

- If a reference value is unavailable, for example before market open or before the feed has populated the instrument, the server may send a zero-filled Circuit Limit or 52-Week packet with `ScripId = 0` and `0.00` values.
- Treat zero-filled reference packets as valid "no data yet" packets. Once values are available, the same packet types are sent with the instrument's `ScripId` and populated values.

## Packed Struct Reference

The following structures map to the wire format. Read all multi-byte values as little-endian and avoid alignment padding when mapping bytes directly.

```text
Header       { uint8 exchSeg; int32 scripId; int32 scripId2; uint8 msgLength; uint8 msgCode }             // 11 bytes
Trade        { Header; float32 ltp; int32 volume; int32 ltt; int32 lut }           // 27 bytes
Ohlc         { Header; float32 fOpen; float32 fClose; float32 fHigh; float32 fLow } // 27 bytes
PClose       { Header; float32 pClose }                                             // 15 bytes
CircuitLimit { Header; float32 uCkt; float32 lCkt }                                 // 19 bytes
Week52       { Header; float32 high; float32 low }                                  // 19 bytes
MarketStatus { Header; byte mktType[3]; int32 mktStatus }                           // 18 bytes
ErrorPacket  { Header; int16 errCode }                                              // 13 bytes
```

## Parsing Example

```python
SIZES = {1: 27, 3: 27, 29: 18, 32: 15, 33: 19, 36: 19, 50: 13}

def on_binary_message(buf):
    offset = 0
    while offset < len(buf):
        msg_length = buf[offset + 9]
        msg_code = buf[offset + 10]
        size = msg_length or SIZES.get(msg_code, 0)

        if size == 0 or offset + size > len(buf):
            break

        packet = buf[offset:offset + size]
        dispatch(msg_code, packet)
        offset += size
```

Parsing and client behavior notes:

- Buffer advancement:
- Use `MsgLength` as the primary value for advancing through the buffer.
- Use the fixed `SIZES` table as a fallback and sanity check.
- Guard against `0` so the parser does not loop forever on malformed data.
- Read-loop handling:
- Keep the read loop active for the lifetime of the connection.
- In Go, a `break` inside a `switch` exits only the `switch`, not the outer parse loop. Use an outer-loop break when stopping on malformed data or an error packet.
- If another foreground service keeps the process alive, handle its startup errors explicitly so a bind failure does not stop the feed reader unexpectedly.
- Field values:
- `LTT` and `LUT` are seconds-based epoch values. Multiply by `1000` when your application needs millisecond epoch timestamps.
- If a time decodes near 1970, the field was likely not populated in that packet yet.

## Quick Reference

| Item | Value |
|------|-------|
| Global feed host | `global-stocks-api-feed.dhan.co` over `wss` |
| Self-hosted port | `7075` over `ws` |
| Frame type | Binary WebSocket frame, opcode `2` |
| Request format | JSON |
| Byte order | Little-endian |
| Supported exchange segment | `INX_EQ`, numeric value `14` |
| Max instruments per request | `100` |
| Max subscriptions per connection | `5000` |
| Max connections per client | `5` |
| Heartbeat | Ping about every 10 seconds; disconnect after 4 missed Pongs |
| Batching | Multiple packets can be concatenated in one WebSocket message |

## Sample Decoded Packets

```text
Header: ExchSeg=14 ScripId=10008660 ScripId2=10008660 MsgLength=27 MsgCode=1
Trade   | ScripId=10008660 LTP=21.39 Vol=33064007 LTT=1466625591 LUT=1782158394

Header: ExchSeg=14 ScripId=10008660 ScripId2=10008660 MsgLength=15 MsgCode=32
PClose  | ScripId=10008660 PClose=21.36

Header: ExchSeg=14 ScripId=10008660 ScripId2=10008660 MsgLength=19 MsgCode=33
CktLmt  | ScripId=10008660 UCkt=27.77 LCkt=14.95

Header: ExchSeg=14 ScripId=10008660 ScripId2=10008660 MsgLength=19 MsgCode=36
52Week  | ScripId=10008660 High=58.15 Low=10.30
```
