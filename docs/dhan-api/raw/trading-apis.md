# Trading APIs

All endpoints for order placement, modification, portfolio management, and trading controls.

---

## Orders

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Place Order](/api/v2/orders/place-order) | Place a new order on the exchange |
| PUT | [Modify Order](/api/v2/orders/modify-order) | Modify a pending order |
| DELETE | [Cancel Order](/api/v2/orders/cancel-order) | Cancel a pending order |
| GET | [Get Order Book](/api/v2/orders/get-orders) | Retrieve all orders for the day |
| GET | [Get Order by ID](/api/v2/orders/get-order-by-id) | Get a specific order by order ID |
| GET | [Get Order by Correlation ID](/api/v2/orders/get-order-by-correlation-id) | Get order by partner correlation ID |
| POST | [Slice Order](/api/v2/orders/slice-order) | Place order with slicing for freeze limit |
| GET | [Get Trade Book](/api/v2/orders/get-trades) | Retrieve all trades for the day |
| GET | [Get Trades for Order](/api/v2/orders/get-trades-by-order-id) | Get trades for a specific order |

## Super Orders

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Place Super Order](/api/v2/super-orders/place-super-order) | Place a bracket order with entry, target, and stop-loss |
| PUT | [Modify Super Order](/api/v2/super-orders/modify-super-order) | Modify a pending super order leg |
| DELETE | [Cancel Super Order Leg](/api/v2/super-orders/cancel-super-order-leg) | Cancel a specific leg of a super order |
| GET | [Get Super Orders](/api/v2/super-orders/get-super-orders) | Retrieve all super orders |

## Conditional and Multi Order

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Place Conditional Order](/api/v2/conditional-triggers/place-conditional-trigger) | Create a new conditional trigger |
| POST | [Place Multi Order](/api/v2/conditional-triggers/place-multi-order) | Place a multi order directly without any conditions |
| PUT | [Modify Conditional Order](/api/v2/conditional-triggers/modify-conditional-trigger) | Modify an existing trigger |
| DELETE | [Delete Conditional Order](/api/v2/conditional-triggers/delete-conditional-trigger) | Delete a trigger |
| GET | [Get Conditional Order by ID](/api/v2/conditional-triggers/get-conditional-trigger-by-id) | Get a specific trigger |
| GET | [Get All Conditional Orders](/api/v2/conditional-triggers/get-all-conditional-triggers) | Retrieve all triggers |

## Portfolio

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [Get Holdings](/api/v2/portfolio/get-holdings) | Retrieve current holdings |
| GET | [Get Positions](/api/v2/portfolio/get-positions) | Retrieve current positions |
| POST | [Convert Position](/api/v2/portfolio/convert-position) | Convert position between product types |
| DELETE | [Exit All Positions](/api/v2/portfolio/exit-all-positions) | Exit all open positions |

## Trader's Control

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Manage Kill Switch](/api/v2/traders-control/manage-kill-switch) | Enable or disable kill switch |
| GET | [Kill Switch Status](/api/v2/traders-control/get-kill-switch-status) | Get current kill switch status |
| POST | [Configure P&L Exit](/api/v2/traders-control/configure-pnl-exit) | Set up P&L based exit |
| GET | [Get P&L Exit](/api/v2/traders-control/get-pnl-exit) | Get current P&L exit config |
| DELETE | [Stop P&L Exit](/api/v2/traders-control/stop-pnl-exit) | Stop P&L based exit |

## EDIS

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [Generate T-PIN](/api/v2/edis/generate-tpin) | Generate T-PIN for EDIS |
| POST | [Generate eDIS Form](/api/v2/edis/generate-edis-form) | Generate eDIS authorization form |
| GET | [EDIS Status Inquiry](/api/v2/edis/edis-inquiry) | Check EDIS authorization status |

## Funds

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [Get Fund Limits](/api/v2/funds/get-fund-limits) | Retrieve available fund limits |
| POST | [Calculate Margin](/api/v2/funds/calculate-margin) | Calculate margin for an order |
| POST | [Calculate Multi-Order Margin](/api/v2/funds/calculate-multi-margin) | Calculate margin for multiple orders |
