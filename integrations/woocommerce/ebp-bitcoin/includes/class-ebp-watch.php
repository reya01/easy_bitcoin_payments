<?php
/**
 * Shop-side tripwire. Runs from the shop's own WordPress cron, so it keeps
 * working even if the payment server is down or misbehaving. It only warns
 * (a notice in the WordPress admin plus one email a day per problem); it never
 * changes orders or settings.
 *
 * @package EBP_Bitcoin
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

class EBP_Watch {

	const HOOK      = 'ebp_bitcoin_watch';
	const ISSUES    = 'ebp_bitcoin_issues';
	const FAILURES  = 'ebp_bitcoin_health_failures';
	const MAX_ORDERS = 500;

	public static function init() {
		add_action( self::HOOK, array( __CLASS__, 'run' ) );
		add_action( 'admin_notices', array( __CLASS__, 'notice' ) );
		if ( ! wp_next_scheduled( self::HOOK ) ) {
			wp_schedule_event( time() + 300, 'hourly', self::HOOK );
		}
	}

	public static function deactivate() {
		wp_clear_scheduled_hook( self::HOOK );
		delete_option( self::ISSUES );
		delete_option( self::FAILURES );
	}

	private static function settings() {
		$s = get_option( 'woocommerce_ebp_bitcoin_settings', array() );
		return is_array( $s ) ? $s : array();
	}

	/** Orders that chose Bitcoin in the last 7 days, and how many of them are paid. */
	public static function order_stats( $now ) {
		$ids = wc_get_orders(
			array(
				'limit'          => self::MAX_ORDERS,
				'return'         => 'ids',
				'payment_method' => 'ebp_bitcoin',
				'date_created'   => '>' . ( $now - 7 * DAY_IN_SECONDS ),
				'status'         => array_keys( wc_get_order_statuses() ),
			)
		);
		$paid = 0;
		foreach ( $ids as $id ) {
			$order = wc_get_order( $id );
			if ( $order && $order->is_paid() ) {
				++$paid;
			}
		}
		return array(
			'created_7d' => count( $ids ),
			'paid_7d'    => $paid,
		);
	}

	/** One check. Returns the list of current problems (also stored for the notice). */
	public static function run( $now = null ) {
		$now = $now ? (int) $now : time();
		$s   = self::settings();
		if ( empty( $s['enabled'] ) || 'yes' !== $s['enabled'] ) {
			delete_option( self::ISSUES );
			return array();
		}

		$client = new EBP_Client( isset( $s['server_url'] ) ? $s['server_url'] : '', isset( $s['api_key'] ) ? $s['api_key'] : '' );
		$health = $client->health();
		$fails  = (int) get_option( self::FAILURES, 0 );
		$fails  = is_wp_error( $health ) ? $fails + 1 : 0;
		update_option( self::FAILURES, $fails, false );

		$stats                    = self::order_stats( $now );
		$stats['server_failures'] = $fails;
		$issues                   = EBP_Core::evaluate_watch( $stats );

		$stored = array();
		foreach ( $issues as $issue ) {
			$stored[ $issue[0] ] = array( $issue[1], $issue[2] );
			$key                 = 'ebp_bitcoin_mailed_' . $issue[0];
			if ( ! get_transient( $key ) ) {
				set_transient( $key, 1, DAY_IN_SECONDS );
				wp_mail(
					get_option( 'admin_email' ),
					'[' . wp_specialchars_decode( get_bloginfo( 'name' ), ENT_QUOTES ) . '] ' . $issue[1],
					$issue[2] . "\n\nThis message comes from the Easy Bitcoin Payments plugin on your shop."
				);
			}
		}
		if ( $stored ) {
			update_option( self::ISSUES, $stored, false );
		} else {
			delete_option( self::ISSUES );
		}
		return $issues;
	}

	public static function notice() {
		if ( ! current_user_can( 'manage_woocommerce' ) ) {
			return;
		}
		$issues = get_option( self::ISSUES, array() );
		if ( ! is_array( $issues ) || ! $issues ) {
			return;
		}
		$url = admin_url( 'admin.php?page=wc-settings&tab=checkout&section=ebp_bitcoin' );
		foreach ( $issues as $issue ) {
			printf(
				'<div class="notice notice-error"><p><strong>%s.</strong> %s <a href="%s">%s</a></p></div>',
				esc_html( $issue[0] ),
				esc_html( $issue[1] ),
				esc_url( $url ),
				esc_html__( 'Bitcoin settings', 'ebp-bitcoin' )
			);
		}
	}
}
