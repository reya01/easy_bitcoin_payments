<?php
/**
 * Pure logic with no WordPress dependencies, so it can be unit tested with
 * plain PHP (see integrations/woocommerce/tests/core-test.php).
 *
 * @package EBP_Bitcoin
 */

if ( ! defined( 'ABSPATH' ) && ! defined( 'EBP_TESTING' ) ) {
	exit;
}

class EBP_Core {

	const SIGNATURE_TOLERANCE_SECS = 300;

	/**
	 * Verify an X-EBP-Signature header: "t=<unix>,v1=<hex HMAC_SHA256(secret, "<t>.<body>")>".
	 * Rejects malformed headers, wrong signatures and stale timestamps.
	 */
	public static function verify_signature( $secret, $header, $body, $now, $tolerance = self::SIGNATURE_TOLERANCE_SECS ) {
		if ( ! is_string( $secret ) || '' === $secret || ! is_string( $header ) ) {
			return false;
		}
		$timestamp = null;
		$signature = null;
		foreach ( explode( ',', $header ) as $part ) {
			$part = trim( $part );
			if ( 0 === strpos( $part, 't=' ) ) {
				$value = substr( $part, 2 );
				if ( '' !== $value && ctype_digit( $value ) ) {
					$timestamp = (int) $value;
				}
			} elseif ( 0 === strpos( $part, 'v1=' ) ) {
				$signature = substr( $part, 3 );
			}
		}
		if ( null === $timestamp || null === $signature || '' === $signature ) {
			return false;
		}
		if ( abs( $now - $timestamp ) > $tolerance ) {
			return false;
		}
		$expected = hash_hmac( 'sha256', $timestamp . '.' . $body, $secret );
		return hash_equals( $expected, $signature );
	}

	/**
	 * What the shop should do for an invoice status reported by the EBP server.
	 * The status always comes from asking the server, never from the webhook alone.
	 *
	 * @return string One of: paid, underpaid, expired, ignore.
	 */
	public static function action_for_status( $status ) {
		switch ( $status ) {
			case 'paid':
			case 'overpaid':
				return 'paid';
			case 'underpaid':
				return 'underpaid';
			case 'expired':
			case 'cancelled':
				return 'expired';
			default:
				return 'ignore';
		}
	}

	/** The order reference we give the server, and how we read it back. */
	public static function order_ref( $order_id ) {
		return 'wc-' . (int) $order_id;
	}

	public static function order_id_from_ref( $ref ) {
		if ( is_string( $ref ) && preg_match( '/^wc-(\d+)$/', $ref, $m ) ) {
			return (int) $m[1];
		}
		return 0;
	}

	/** Money as a plain decimal string with the shop currency's number of decimals. */
	public static function format_amount( $total, $decimals ) {
		return number_format( (float) $total, max( 0, (int) $decimals ), '.', '' );
	}

	/** Trim, drop trailing slashes, accept only http(s) URLs. Empty string when invalid. */
	public static function normalize_server_url( $url ) {
		$url = rtrim( trim( (string) $url ), '/' );
		if ( ! preg_match( '#^https?://[^\s/]+#i', $url ) ) {
			return '';
		}
		return $url;
	}

	/** Plain http is only acceptable for a server on the same machine. */
	public static function is_secure_enough( $url ) {
		if ( 0 === stripos( $url, 'https://' ) ) {
			return true;
		}
		$host = strtolower( (string) parse_url( $url, PHP_URL_HOST ) );
		return in_array( $host, array( 'localhost', '127.0.0.1', '::1', '[::1]' ), true );
	}

	/** Body for POST /api/v1/invoices. */
	public static function invoice_request( $order_id, $currency, $total, $decimals, $expiry_minutes = null ) {
		$body = array(
			'order_id' => self::order_ref( $order_id ),
			'fiat'     => array(
				'currency' => strtoupper( (string) $currency ),
				'amount'   => self::format_amount( $total, $decimals ),
			),
			'metadata' => array( 'source' => 'woocommerce' ),
		);
		if ( $expiry_minutes ) {
			$body['expiry_minutes'] = (int) $expiry_minutes;
		}
		return $body;
	}
}
