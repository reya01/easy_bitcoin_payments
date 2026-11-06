<?php
/**
 * Talks to the merchant's own Easy Bitcoin Payments server.
 *
 * @package EBP_Bitcoin
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

class EBP_Client {

	private $server;
	private $api_key;

	public function __construct( $server, $api_key ) {
		$this->server  = EBP_Core::normalize_server_url( $server );
		$this->api_key = (string) $api_key;
	}

	private function request( $method, $path, $body = null, $auth = true ) {
		if ( '' === $this->server ) {
			return new WP_Error( 'ebp_config', 'The payment server address is not set.' );
		}
		$args = array(
			'method'  => $method,
			'timeout' => 15,
			'headers' => array( 'Accept' => 'application/json' ),
		);
		if ( $auth ) {
			$args['headers']['Authorization'] = 'Bearer ' . $this->api_key;
		}
		if ( null !== $body ) {
			$args['headers']['Content-Type'] = 'application/json';
			$args['body']                    = wp_json_encode( $body );
		}
		$res = wp_remote_request( $this->server . $path, $args );
		if ( is_wp_error( $res ) ) {
			return $res;
		}
		$code = (int) wp_remote_retrieve_response_code( $res );
		$json = json_decode( wp_remote_retrieve_body( $res ), true );
		if ( $code < 200 || $code >= 300 ) {
			$msg = 'The payment server answered ' . $code . '.';
			if ( is_array( $json ) && isset( $json['error']['message'] ) ) {
				$msg = (string) $json['error']['message'];
			}
			return new WP_Error( 'ebp_http_' . $code, $msg, array( 'status' => $code ) );
		}
		return is_array( $json ) ? $json : array();
	}

	public function create_invoice( array $body ) {
		return $this->request( 'POST', '/api/v1/invoices', $body );
	}

	public function get_invoice( $id ) {
		return $this->request( 'GET', '/api/v1/invoices/' . rawurlencode( $id ) );
	}

	/** No sign-in needed: is the server up? */
	public function health() {
		return $this->request( 'GET', '/api/v1/health', null, false );
	}

	/** Does the API key work? */
	public function check_key() {
		return $this->request( 'GET', '/api/v1/invoices?limit=1' );
	}
}
