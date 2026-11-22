# Trader's Control

Trader's Control: trading kill switch and P&L based auto exit (daily profit target and max loss) with the Dhan API.

These set of APIs are built for traders to manage their risks and preferences using advanced tools built in right into Dhan. You can set and manage Kill Switch for your account along with having P&L based auto-exit feature.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/killswitch`](/api/v2/traders-control/manage-kill-switch) | Manage Kill Switch |
| GET | [`/killswitch`](/api/v2/traders-control/get-kill-switch-status) | Kill Switch Status |
| POST | [`/pnlExit`](/api/v2/traders-control/configure-pnl-exit) | Configure P&L Based Exit |
| DELETE | [`/pnlExit`](/api/v2/traders-control/stop-pnl-exit) | Stop P&L Based Exit |
| GET | [`/pnlExit`](/api/v2/traders-control/get-pnl-exit) | Get P&L Based Exit |
