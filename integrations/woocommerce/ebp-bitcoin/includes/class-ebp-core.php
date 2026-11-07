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

	/**
	 * Does the invoice still match the order? The order total can change after
	 * the invoice was made (an admin edits the order, a coupon is added); a payment
	 * for the old amount must not silently complete the changed order.
	 */
	public static function amounts_match( $inv_currency, $inv_amount, $currency, $total, $decimals ) {
		if ( ! is_string( $inv_currency ) || '' === $inv_currency || ! is_numeric( $inv_amount ) ) {
			return false;
		}
		if ( strtoupper( $inv_currency ) !== strtoupper( (string) $currency ) ) {
			return false;
		}
		$half_unit = 0.5 / pow( 10, max( 0, (int) $decimals ) );
		return abs( (float) $inv_amount - (float) $total ) < $half_unit;
	}

	/** Body for POST /api/v1/invoices. */
	public static function invoice_request( $order_id, $currency, $total, $decimals, $expiry_minutes = null, $return_url = '' ) {
		$body = array(
			'order_id' => self::order_ref( $order_id ),
			'fiat'     => array(
				'currency' => strtoupper( (string) $currency ),
				'amount'   => self::format_amount( $total, $decimals ),
			),
			'metadata' => array( 'source' => 'woocommerce' ),
		);
		if ( is_string( $return_url ) && preg_match( '#^https?://[^\s]+$#i', $return_url ) ) {
			$body['return_url'] = $return_url;
		}
		if ( $expiry_minutes ) {
			$body['expiry_minutes'] = (int) $expiry_minutes;
		}
		return $body;
	}

	const WATCH_MIN_ORDERS    = 5; // Bitcoin orders in a week, none paid, before we warn.
	const WATCH_MIN_FAILURES  = 3; // Hourly server checks in a row that must fail.

	/**
	 * Shop-side tripwire: things the shop can notice WITHOUT trusting the
	 * payment server. Returns a list of array( code, title, detail ).
	 *
	 * @param array $stats created_7d, paid_7d, server_failures.
	 */
	public static function evaluate_watch( array $stats ) {
		$issues  = array();
		$created = isset( $stats['created_7d'] ) ? (int) $stats['created_7d'] : 0;
		$paid    = isset( $stats['paid_7d'] ) ? (int) $stats['paid_7d'] : 0;
		$fails   = isset( $stats['server_failures'] ) ? (int) $stats['server_failures'] : 0;

		if ( $created >= self::WATCH_MIN_ORDERS && 0 === $paid ) {
			$issues[] = array(
				'no_payments',
				'Bitcoin orders are not getting paid',
				sprintf( '%d customers chose Bitcoin in the last 7 days and none of those orders was marked paid. Check your own wallet. If it received money, the connection is broken; if not, something may be wrong with your payment server. You can turn Bitcoin off in WooCommerce > Settings > Payments while you look.', $created ),
			);
		}
		if ( $fails >= self::WATCH_MIN_FAILURES ) {
			$issues[] = array(
				'server_down',
				'Your payment server is not answering',
				sprintf( 'Your shop could not reach your payment server on the last %d hourly checks. Customers who choose Bitcoin will not be able to pay until it is back.', $fails ),
			);
		}
		return $issues;
	}
}
