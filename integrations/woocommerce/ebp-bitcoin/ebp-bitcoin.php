<?php
/**
 * Plugin Name:       Easy Bitcoin Payments for WooCommerce
 * Description:       Accept Bitcoin in WooCommerce through your own Easy Bitcoin Payments server.
 * Version:           0.1.0
 * Requires at least: 6.2
 * Requires PHP:      7.4
 * License:           MIT
 * Text Domain:       ebp-bitcoin
 * WC requires at least: 8.0
 *
 * @package EBP_Bitcoin
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

define( 'EBP_BITCOIN_VERSION', '0.1.0' );
define( 'EBP_BITCOIN_FILE', __FILE__ );

add_action(
	'before_woocommerce_init',
	function () {
		if ( class_exists( \Automattic\WooCommerce\Utilities\FeaturesUtil::class ) ) {
			\Automattic\WooCommerce\Utilities\FeaturesUtil::declare_compatibility( 'custom_order_tables', __FILE__, true );
			\Automattic\WooCommerce\Utilities\FeaturesUtil::declare_compatibility( 'cart_checkout_blocks', __FILE__, true );
		}
	}
);

add_action(
	'plugins_loaded',
	function () {
		if ( ! class_exists( 'WC_Payment_Gateway' ) ) {
			return;
		}
		require_once __DIR__ . '/includes/class-ebp-core.php';
		require_once __DIR__ . '/includes/class-ebp-client.php';
		require_once __DIR__ . '/includes/class-ebp-gateway.php';
		require_once __DIR__ . '/includes/class-ebp-watch.php';
		EBP_Watch::init();

		add_filter(
			'woocommerce_payment_gateways',
			function ( $gateways ) {
				$gateways[] = 'EBP_Gateway';
				return $gateways;
			}
		);
		add_action(
			'woocommerce_thankyou_ebp_bitcoin',
			function ( $order_id ) {
				( new EBP_Gateway() )->maybe_show_pay_link( $order_id );
			}
		);
		add_action(
			'woocommerce_view_order',
			function ( $order_id ) {
				( new EBP_Gateway() )->maybe_show_pay_link( $order_id );
			}
		);
	},
	11
);

add_action(
	'woocommerce_blocks_loaded',
	function () {
		if ( ! class_exists( 'Automattic\WooCommerce\Blocks\Payments\Integrations\AbstractPaymentMethodType' ) ) {
			return;
		}
		require_once __DIR__ . '/includes/class-ebp-blocks.php';
		add_action(
			'woocommerce_blocks_payment_method_type_registration',
			function ( $registry ) {
				$registry->register( new EBP_Blocks() );
			}
		);
	}
);

register_deactivation_hook(
	__FILE__,
	function () {
		require_once __DIR__ . '/includes/class-ebp-watch.php';
		EBP_Watch::deactivate();
	}
);
