# DhanHQ API

Introduction to DhanHQ Trading & Investment API

# Getting Started

DhanHQ API is a state-of-the-art platform that enables you to build trading and investment services, applications, and strategies.

It provides a set of REST-like APIs that integrate directly with our trading platform. With DhanHQ APIs, you can execute and modify orders in real time, manage portfolios, access live market data, and more through a fast and reliable API collection.

Start Trading & Investing with Superfast APIs

The platform uses resource-based URLs and supports JSON as well as form-encoded requests. Responses are returned in JSON format using standard HTTP response codes, verbs, and authentication methods.

[DhanHQ Python Client pip install dhanhq View on GitHub](https://github.com/dhan-oss/DhanHQ-py)

## Built for algo traders

Whether you run your own strategies, backtest ideas, automate your setup or need reliable market data and fast trading APIs, DhanHQ APIs cover the full workflow.

Trade your strategies

Backtest your ideas

Automate your setup

Get reliable market data

Businesses built on DhanHQ

Fintechs
Banks
Wealth Platforms
Sub Brokers
AI & Agents
Trading Tools
Algo Providers
& more

## Structure

#### REST

All **GET** and **DELETE** request parameters go as query parameters, and **POST** and **PUT** parameters as form-encoded. User has to input an access token in the header for every request.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/ \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{Request JSON}'
```

#### Python

Install Python Package directly using following command in command line.

```bash title="Terminal"
pip install dhanhq
```
This installs entire DhanHQ Python Client along with the required packages. Now, you can start using DhanHQ Client with your Python script.

You can now import 'dhanhq' module and connect to your Dhan account.

```python title="Python"
from dhanhq import dhanhq

dhan = dhanhq("client_id","access_token")
```

## Errors

Error responses come with the error code and message generated internally by the system. The sample structure of error response is shown below.

```json title="Error response"
{
    "errorType": "",
    "errorCode": "",
    "errorMessage": ""
}
```

You can find detailed error code and message under [Annexure](/api/v2/annexure).

## Rate Limits

| API Category | Rate | Per-request cap | Daily limit |
|--------------|------|-----------------|-------------|
| **Order APIs** | 10 req/sec | — | 7,000 |
| **Data APIs** | 5 req/sec | — | 100,000 |
| **Market Quote** | 1 req/sec | 1,000 instruments | — |
| **Option Chain** | 1 req / 3 sec
10 unique scrips / 8 sec | 1 scrip | — |
| **Non Trading APIs** | 20 req/sec
Unlimited per minute and per hour | — | Unlimited |
