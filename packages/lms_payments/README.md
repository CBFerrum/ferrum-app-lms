# lms_payments — Paid Courses

Package `lms_payments` (depends on `lms` and `commerce`; logic in `src/payments.rs` of the
`ferrum-app-lms` crate), shipped as the connector app **Paid Courses** (`connectors/payments/app.yaml`).
It sells courses through the store's payment methods.

## Setup

| Where | Field | Meaning |
|---|---|---|
| Learn Settings | `lpay_company` | The company that invoices paid courses (its currency is the courses') |
| Learn Settings | `lpay_payment_method` | The store payment method (a gateway plugin's name, as in Store Settings) learners pay through. Empty: courses cannot be bought |
| Learn Settings | `lpay_refund_days` | Default 14: withdrawing within this many days of paying, before completing the course, refunds it whole. 0: never |
| Course | `lpay_price` | Empty or 0: free. More: learners buy it before they are enrolled |
| Course | `lpay_item` | The item its invoices carry (required with a price): its income account and taxes apply |

## Buying

`POST /api/v1/LmsCourse/{id}/actions/buy` (any Learn role, on a published course open to
self-enrolment whose prerequisites the caller has completed) answers
`{"purchase": {id, number, status, amount, paid_at, payment: {status, payment_url}}}`:

1. It makes a **Course Purchase** (`LmsCoursePurchase`, `Awaiting Payment`) and a submitted sales
   invoice for the learner's customer (an enabled customer with their email, or a new individual).
2. It asks for the whole invoice through the payment method (a Commerce `PaymentRequest`). The
   gateway plugin writes `payment_url`, which the course page links to.
3. When the gateway confirms the payment (`mark_paid`), the payment entry marks the purchase `Paid`
   and enrols the learner, or reactivates the enrolment they had left, with their progress.

Calling it again while the payment is due returns the same purchase. If the request lapsed or
failed, it asks for the same invoice again. Maintenance cancels a purchase whose request lapsed,
failed or was cancelled more than a day ago, and its invoice while nothing was paid on it.

## Rules

- A learner cannot enrol themselves in a paid course without a `Paid` purchase. This applies to new
  enrolments, Enrol Again and joining a learning path. Its instructors and LMS managers enrol anyone.
- Withdrawing within the refund period credits the invoice in full (a submitted credit note) and
  asks for the payment back (`OrderRefund`, kind Payment, Requested). The purchase becomes
  `Refund Requested`, and a person approves the refund in the store. After the period, the purchase
  stays `Paid` and Enrol Again needs no new purchase.
- Learners read their own purchases. Instructors read all of them, and LMS managers read and export them.
