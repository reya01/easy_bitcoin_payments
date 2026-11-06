<?php
// Plain-PHP tests for the logic that needs no WordPress. Run: php core-test.php
define( 'EBP_TESTING', true );
require __DIR__ . '/../ebp-bitcoin/includes/class-ebp-core.php';

$fail = 0;
function check( $name, $ok ) {
	global $fail;
	echo ( $ok ? 'PASS  ' : 'FAIL  ' ) . $name . "\n";
	if ( ! $ok ) {
		$fail++;
	}
}

$secret = 'whsec_test';
$body   = '{"event":"invoice.paid","invoice_id":"inv_1"}';
$now    = 1000000;
$sig    = 't=' . $now . ',v1=' . hash_hmac( 'sha256', $now . '.' . $body, $secret );

check( 'good signature is accepted', EBP_Core::verify_signature( $secret, $sig, $body, $now ) );
check( 'whitespace after the comma is fine', EBP_Core::verify_signature( $secret, str_replace( ',', ', ', $sig ), $body, $now ) );
check( 'tampered body is rejected', ! EBP_Core::verify_signature( $secret, $sig, $body . ' ', $now ) );
check( 'wrong secret is rejected', ! EBP_Core::verify_signature( 'other', $sig, $body, $now ) );
check( 'empty secret is rejected', ! EBP_Core::verify_signature( '', $sig, $body, $now ) );
check( 'stale timestamp is rejected', ! EBP_Core::verify_signature( $secret, $sig, $body, $now + 301 ) );
check( 'timestamp inside the window is accepted', EBP_Core::verify_signature( $secret, $sig, $body, $now + 300 ) );
check( 'garbage header is rejected', ! EBP_Core::verify_signature( $secret, 'nonsense', $body, $now ) );
check( 'missing v1 is rejected', ! EBP_Core::verify_signature( $secret, 't=' . $now, $body, $now ) );
check( 'non-numeric timestamp is rejected', ! EBP_Core::verify_signature( $secret, 't=abc,v1=00', $body, $now ) );

check( 'paid maps to paid', 'paid' === EBP_Core::action_for_status( 'paid' ) );
check( 'overpaid counts as paid', 'paid' === EBP_Core::action_for_status( 'overpaid' ) );
check( 'underpaid is its own case', 'underpaid' === EBP_Core::action_for_status( 'underpaid' ) );
check( 'expired and cancelled both close the order', 'expired' === EBP_Core::action_for_status( 'expired' ) && 'expired' === EBP_Core::action_for_status( 'cancelled' ) );
check( 'pending is ignored', 'ignore' === EBP_Core::action_for_status( 'pending' ) );
check( 'unknown is ignored', 'ignore' === EBP_Core::action_for_status( 'whatever' ) );

check( 'order ref round trip', 42 === EBP_Core::order_id_from_ref( EBP_Core::order_ref( 42 ) ) );
check( 'foreign order ref is rejected', 0 === EBP_Core::order_id_from_ref( 'shopify-9' ) && 0 === EBP_Core::order_id_from_ref( 'wc-9x' ) && 0 === EBP_Core::order_id_from_ref( null ) );

check( 'amount keeps two decimals', '12.50' === EBP_Core::format_amount( 12.5, 2 ) );
check( 'amount has no thousands separator', '1234.00' === EBP_Core::format_amount( '1234', 2 ) );
check( 'zero-decimal currency', '1500' === EBP_Core::format_amount( 1500, 0 ) );

check( 'server url is trimmed', 'https://pay.example.com' === EBP_Core::normalize_server_url( " https://pay.example.com/ \n" ) );
check( 'non-http server url is rejected', '' === EBP_Core::normalize_server_url( 'ftp://x' ) && '' === EBP_Core::normalize_server_url( 'pay.example.com' ) );
check( 'https is secure', EBP_Core::is_secure_enough( 'https://pay.example.com' ) );
check( 'plain http to a remote host is not', ! EBP_Core::is_secure_enough( 'http://pay.example.com' ) );
check( 'plain http to localhost is allowed', EBP_Core::is_secure_enough( 'http://127.0.0.1:8080' ) );

$req = EBP_Core::invoice_request( 7, 'usd', 19.9, 2 );
check( 'invoice request carries ref, currency and amount', 'wc-7' === $req['order_id'] && 'USD' === $req['fiat']['currency'] && '19.90' === $req['fiat']['amount'] && ! isset( $req['expiry_minutes'] ) );

$req = EBP_Core::invoice_request( 7, 'USD', 5, 2, null, 'https://shop.example.com/order-received/7/?key=k' );
check( 'return link is passed along', 'https://shop.example.com/order-received/7/?key=k' === $req['return_url'] );
$req = EBP_Core::invoice_request( 7, 'USD', 5, 2, null, 'javascript:alert(1)' );
check( 'a non-http return link is dropped', ! isset( $req['return_url'] ) );

echo $fail ? "\n$fail failed\n" : "\nall passed\n";
exit( $fail ? 1 : 0 );
