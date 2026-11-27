<?php
// Plain-PHP test of the hourly check's answer to server errors, with small
// stand-ins for WordPress, WooCommerce and the server client.
// Run: php hourly-check-test.php
define( 'ABSPATH', __DIR__ );
define( 'EBP_TESTING', true );

$logged = array();
function __( $s ) {
	return $s;
}
class WC_Payment_Gateway {
	public function get_option( $k ) {
		return 'api_key' === $k ? 'k' : 'https://pay.example.com';
	}
}
class WP_Error {
	private $data;
	public function __construct( $code, $msg, $data ) {
		$this->data = $data;
	}
	public function get_error_data() {
		return $this->data;
	}
	public function get_error_message() {
		return 'refused';
	}
}
function is_wp_error( $x ) {
	return $x instanceof WP_Error;
}
class Logger {
	public function log( $level, $message ) {
		global $logged;
		$logged[] = $message;
	}
}
function wc_get_logger() {
	return new Logger();
}
class EBP_Client {
	public static $status = 0;
	public function get_invoice( $id ) {
		return new WP_Error( 'http', 'refused', array( 'status' => self::$status ) );
	}
}
class Order {
	public function get_meta( $k ) {
		return 'inv_1';
	}
	public function get_id() {
		return 7;
	}
}
require __DIR__ . '/../ebp-bitcoin/includes/class-ebp-core.php';
require __DIR__ . '/../ebp-bitcoin/includes/class-ebp-gateway.php';

$fail = 0;
function check( $name, $ok ) {
	global $fail;
	echo ( $ok ? 'PASS  ' : 'FAIL  ' ) . $name . "\n";
	if ( ! $ok ) {
		$fail++;
	}
}

$gateway = ( new ReflectionClass( 'EBP_Gateway' ) )->newInstanceWithoutConstructor();
$answer  = function ( $status ) use ( $gateway ) {
	global $logged;
	$logged             = array();
	EBP_Client::$status = $status;
	return $gateway->check_with_server( new Order() );
};

foreach ( array( 401, 403, 429 ) as $code ) {
	check( "HTTP $code stops the hourly check (asked again next hour)", false === $answer( $code ) );
	check( "HTTP $code is logged as a connection problem, not an unknown invoice", 1 === count( $logged ) && false !== strpos( $logged[0], 'API key' ) && false === strpos( $logged[0], 'does not know' ) );
}
check( 'HTTP 404 skips only this order', true === $answer( 404 ) );
check( 'HTTP 404 says the server does not know the invoice', false !== strpos( $logged[0], 'does not know invoice inv_1' ) );
check( 'HTTP 400 skips only this order', true === $answer( 400 ) );
check( 'no answer stops the hourly check', false === $answer( 0 ) );
check( 'a server error stops the hourly check', false === $answer( 502 ) );

exit( $fail ? 1 : 0 );
