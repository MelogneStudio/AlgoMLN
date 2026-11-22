# Postback

Webhooks for real-time order status updates via POST callbacks

**Postback** (also called **Webhooks**) sends order updates to a URL you provide. Whenever an order changes, a `POST` request with a JSON payload is sent to your Postback URL.

### What it does

- Sends an order update when the status of an order changes: `TRANSIT`, `PENDING`, `REJECTED`, `CANCELLED`, `TRADED` or `EXPIRED`.
- Sends an update whenever an order is modified.
- Sends an update whenever an order is partially filled.
- Works at the access token level: all trades originating from one access token are sent to the Webhook URL set for that token.

**How it works**

1. While generating an access token on [web.dhan.co](https://web.dhan.co), enter your URL in the **Postback URL** field and click **Generate**.
2. Place orders using that access token.
3. When an order's status changes, or it is modified or partially filled, a `POST` request with a JSON payload is sent to your Postback URL.
4. The JSON payload is sent as a raw HTTP POST body.

See the [Postback Payload](#postback-payload) for the JSON structure and [Setting Up Postback](#setting-up-postback) for the full setup.

---

Postback API (Webhooks) sends a `POST` request with **JSON payload** to your Postback URL whenever there is a change in order status (`TRANSIT`, `PENDING`, `REJECTED`, `CANCELLED`, `TRADED` or `EXPIRED`) or whenever an order is modified or partially filled.

This Postback API works on **access token** level — all trades originating from one particular access token are sent to that particular Webhook URL. This makes it optimal for individual developers.

---

## Postback Payload

The JSON payload is sent as a raw HTTP POST body.

```json
{
    "dhanClientId": "1000000003",
    "orderId": "112111182198",
    "correlationId": "123abc678",
    "orderStatus": "PENDING",
    "transactionType": "BUY",
    "exchangeSegment": "NSE_EQ",
    "productType": "INTRADAY",
    "orderType": "MARKET",
    "validity": "DAY",
    "tradingSymbol": "",
    "securityId": "11536",
    "quantity": 5,
    "disclosedQuantity": 0,
    "price": 0.0,
    "triggerPrice": 0.0,
    "afterMarketOrder": false,
    "boProfitValue": 0.0,
    "boStopLossValue": 0.0,
    "legName": null,
    "createTime": "2021-11-24 13:33:03",
    "updateTime": "2021-11-24 13:33:03",
    "exchangeTime": "2021-11-24 13:33:03",
    "drvExpiryDate": null,
    "drvOptionType": null,
    "drvStrikePrice": 0.0,
    "omsErrorCode": null,
    "omsErrorDescription": null,
    "filled_qty": 1,
    "algoId": null
}
```

---

## Setting Up Postback

To set up Postback API:

1. While generating access token on [web.dhan.co](https://web.dhan.co), enter your URL in the **'Postback URL'** field
2. Click on **'Generate'** to set Postback and generate a new token

> **warning:** You will not receive postback calls if Postback URL is set to `localhost` or `127.0.0.1`.

> **note:** To receive Postback for all orders placed from a platform/app, [Partner Login](/api/v2/authentication#for-partners) module needs to be used.

For description of enum values, refer to the [Annexure](/api/v2/annexure).


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| dhanClientId | string | No | User specific identification |
| orderId | string | No | Order specific identification by Dhan |
| correlationId | string | No | User/partner generated tracking ID (max 30 chars, ) Values: `[a-zA-Z0-9 _-]`. |
| orderStatus | enum | No | Last updated status of the order see [annexure](/api/v2/annexure#order-status) Values: `TRANSIT`, `PENDING`, `REJECTED`, `CANCELLED`, `TRADED`, `EXPIRED`. |
| transactionType | enum | No | or Values: `BUY`, `SELL`. |
| exchangeSegment | enum | No | Exchange & Segment see [annexure](/api/v2/annexure#exchange-segment) Values: `NSE_EQ`, `NSE_FNO`, `NSE_CURRENCY`, `BSE_EQ`, `MCX_COMM`. |
| productType | enum | No | Product type of trade see [annexure](/api/v2/annexure#product-type) Values: `CNC`, `INTRADAY`, `MARGIN`, `MTF`. |
| orderType | enum | No | Values: `LIMIT`, `MARKET`, `STOP_LOSS`, `STOP_LOSS_MARKET`. |
| validity | enum | No | or Values: `DAY`, `IOC`. |
| tradingSymbol | string | No | Trading Symbol |
| securityId | string | No | Exchange standard ID ([Instruments](/api/v2/instruments)) |
| quantity | int | No | Number of shares for the order |
| disclosedQuantity | int | No | Number of shares visible |
| price | float | No | Price at which order is placed |
| triggerPrice | float | No | Trigger price for SL-M, SL-L, CO & BO |
| afterMarketOrder | boolean | No | Whether the order is AMO |
| boProfitValue | float | No | Bracket Order Target Price change |
| boStopLossValue | float | No | Bracket Order Stop Loss Price change |
| legName | enum | No | Values: `ENTRY_LEG`, `TARGET_LEG`, `STOP_LOSS_LEG`. |
| createTime | string | No | Time at which order is created |
| updateTime | string | No | Time at which last activity happened |
| exchangeTime | string | No | Time at which order reached exchange |
| drvExpiryDate | string | No | F&O contract expiry date |
| drvOptionType | enum | No | or Values: `CALL`, `PUT`. |
| drvStrikePrice | float | No | Options Strike Price |
| omsErrorCode | string | No | Error code in case the order is rejected or failed |
| omsErrorDescription | string | No | Description of error in case the order is rejected or failed |
| filled_qty | int | No | Quantity already traded |
| algoId | string | No | Algo ID registered with Exchange |
