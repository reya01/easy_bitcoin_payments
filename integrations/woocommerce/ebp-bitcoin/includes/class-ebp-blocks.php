<?php
/**
 * Makes the Bitcoin method show up in the block-based checkout.
 *
 * @package EBP_Bitcoin
 */

use Automattic\WooCommerce\Blocks\Payments\Integrations\AbstractPaymentMethodType;

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

final class EBP_Blocks extends AbstractPaymentMethodType {

	protected $name = 'ebp_bitcoin';

	public function initialize() {
		$this->settings = get_option( 'woocommerce_ebp_bitcoin_settings', array() );
	}

	public function is_active() {
		return ! empty( $this->settings['enabled'] ) && 'yes' === $this->settings['enabled'];
	}

	public function get_payment_method_script_handles() {
		wp_register_script(
			'ebp-bitcoin-blocks',
			plugins_url( 'assets/blocks.js', EBP_BITCOIN_FILE ),
			array( 'wc-blocks-registry', 'wc-settings', 'wp-element', 'wp-html-entities' ),
			EBP_BITCOIN_VERSION,
			true
		);
		return array( 'ebp-bitcoin-blocks' );
	}

	public function get_payment_method_data() {
		return array(
			'title'       => isset( $this->settings['title'] ) ? $this->settings['title'] : 'Bitcoin',
			'description' => isset( $this->settings['description'] ) ? $this->settings['description'] : '',
			'supports'    => array( 'products' ),
		);
	}
}
