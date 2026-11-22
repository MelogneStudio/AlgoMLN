# Setup Static IP

Set, modify and view the static IP whitelisted on your Dhan account.

Static IP whitelisting is mandatory as per the new SEBI and exchange guidelines.

In line with this, you can use the APIs below to set Static IP for your account, or use Dhan Web to set it up manually.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/ip/setIP`](/api/v2/authentication/set-ip) | Set the primary or secondary static IP |
| PUT | [`/ip/modifyIP`](/api/v2/authentication/modify-ip) | Modify the primary or secondary static IP |
| GET | [`/ip/getIP`](/api/v2/authentication/get-ip) | Get the primary and secondary static IP and their modification dates |

Do note that each individual needs to have a unique static IP. Once an IP is whitelisted, it cannot be edited for the next 7 days or as recommended by the exchange.

Do note that Static IP is only required while using Order Placement APIs including Orders and Super Order. While fetching order details or trade details, no such IP whitelisting is required.

> **info:** A static IP is a fixed, permanent internet address for your device or server. Unlike the default IP on home Wi-Fi, a static IP does not change. To use one, you need to request and purchase it separately from your Internet Service Provider (ISP).
