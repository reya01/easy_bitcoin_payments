<?php
/**
 * The "Bitcoin" payment method.
 *
 * @package EBP_Bitcoin
 */

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

class EBP_Gateway extends WC_Payment_Gateway {

	public function __construct() {
		$this->id                 = 'ebp_bitcoin';
		$this->method_title       = 'Bitcoin (Easy Bitcoin Payments)';
		$this->method_description = 'Accept Bitcoin through your own Easy Bitcoin Payments server. Customers pay on a hosted checkout page; the order is marked paid automatically.';
		$this->has_fields         = false;
		$this->supports           = array( 'products' );

		$this->init_form_fields();
		$this->init_settings();

		$this->title       = $this->get_option( 'title', 'Bitcoin' );
		$this->description = $this->get_option( 'description', 'Pay with Bitcoin (Lightning, Ark or on-chain).' );

		add_action( 'woocommerce_update_options_payment_gateways_' . $this->id, array( $this, 'process_admin_options' ) );
		add_action( 'woocommerce_api_' . $this->id, array( $this, 'handle_webhook' ) );
	}

	public function webhook_url() {
		return add_query_arg( 'wc-api', $this->id, home_url( '/' ) );
	}

	public function init_form_fields() {
		$this->form_fields = array(
			'enabled'        => array(
				'title'   => 'Enable',
				'type'    => 'checkbox',
				'label'   => 'Accept Bitcoin',
				'default' => 'no',
			),
			'server_url'     => array(
				'title'       => 'Server address',
				'type'        => 'text',
				'description' => 'Your payment server, from the "Connect your shop" tab of its dashboard. Example: https://pay.myshop.com',
				'desc_tip'    => false,
				'default'     => '',
				'placeholder' => 'https://',
			),
			'api_key'        => array(
				'title'       => 'API key',
				'type'        => 'password',
				'description' => 'Make one in the dashboard: Connect your shop, step 2.',
				'default'     => '',
			),
			'webhook_secret' => array(
				'title'       => 'Signing secret',
				'type'        => 'password',
				'description' => 'From the dashboard: Connect your shop, step 3 (shown once when you save the address).',
				'default'     => '',
			),
			'webhook_info'   => array(
				'title'       => 'Your shop\'s notification address',
				'type'        => 'title',
				'description' => 'Paste this into the dashboard (Connect your shop, step 3):<br><code>' . esc_html( $this->webhook_url() ) . '</code>',
			),
			'title'          => array(
				'title'   => 'Name shown to customers',
				'type'    => 'text',
				'default' => 'Bitcoin',
			),
			'description'    => array(
				'title'   => 'Message shown to customers',
				'type'    => 'textarea',
				'default' => 'Pay with Bitcoin (Lightning, Ark or on-chain).',
			),
		);
	}

	private function client() {
		return new EBP_Client( $this->get_option( 'server_url' ), $this->get_option( 'api_key' ) );
	}

	public function is_available() {
		if ( 'yes' !== $this->enabled ) {
			return false;
		}
		return '' !== EBP_Core::normalize_server_url( $this->get_option( 'server_url' ) )
			&& '' !== $this->get_option( 'api_key' );
	}

	/** After saving, tell the merchant plainly whether the connection works. */
	public function process_admin_options() {
		$saved = parent::process_admin_options();

		$url = EBP_Core::normalize_server_url( $this->get_option( 'server_url' ) );
		if ( '' === $url ) {
			WC_Admin_Settings::add_error( 'Bitcoin: the server address is missing or does not start with https://' );
			return $saved;
		}
		if ( ! EBP_Core::is_secure_enough( $url ) ) {
			WC_Admin_Settings::add_error( 'Bitcoin: the server address should start with https:// so your API key is not sent in the clear.' );
		}
		$client = new EBP_Client( $url, $this->get_option( 'api_key' ) );
		$health = $client->health();
		if ( is_wp_error( $health ) ) {
			WC_Admin_Settings::add_error( 'Bitcoin: could not reach your payment server (' . $health->get_error_message() . ').' );
			return $saved;
		}
		$key = $client->check_key();
		if ( is_wp_error( $key ) ) {
			WC_Admin_Settings::add_error( 'Bitcoin: the server answered, but the API key was not accepted. Make a new one in the dashboard and paste it here.' );
			return $saved;
		}
		if ( '' === $this->get_option( 'webhook_secret' ) ) {
			WC_Admin_Settings::add_error( 'Bitcoin: connected, but the signing secret is empty, so paid orders cannot be marked paid automatically.' );
			return $saved;
		}
		WC_Admin_Settings::add_message( 'Bitcoin: connected to your payment server.' );
		return $saved;
	}

	public function process_payment( $order_id ) {
		$order = wc_get_order( $order_id );
		if ( ! $order ) {
			return array( 'result' => 'failure' );
		}

		$existing = $order->get_meta( '_ebp_checkout_url' );
		$body     = EBP_Core::invoice_request(
			$order->get_id(),
			$order->get_currency(),
			$order->get_total(),
			wc_get_price_decimals(),
			null,
			$this->get_return_url( $order )
		);
		// The server returns the same open invoice for a repeated order reference.
		$invoice = $this->client()->create_invoice( $body );
		if ( is_wp_error( $invoice ) || empty( $invoice['checkout_url'] ) || empty( $invoice['id'] ) ) {
			$detail = is_wp_error( $invoice ) ? $invoice->get_error_message() : 'unexpected answer';
			wc_get_logger()->error( 'Could not create a Bitcoin invoice for order ' . $order->get_id() . ': ' . $detail, array( 'source' => 'ebp-bitcoin' ) );
			wc_add_notice( 'Bitcoin payment is not available right now. Please choose another method or try again shortly.', 'error' );
			return array( 'result' => 'failure' );
		}

		$order->update_meta_data( '_ebp_invoice_id', (string) $invoice['id'] );
		$order->update_meta_data( '_ebp_checkout_url', (string) $invoice['checkout_url'] );
		$order->update_status( 'on-hold', 'Waiting for the Bitcoin payment.' );
		if ( ! $existing ) {
			wc_reduce_stock_levels( $order->get_id() );
		}
		$order->save();
		WC()->cart->empty_cart();

		return array(
			'result'   => 'success',
			'redirect' => (string) $invoice['checkout_url'],
		);
	}

	/** Customers who return to an unpaid order get a link back to the payment page. */
	public function maybe_show_pay_link( $order_id ) {
		$order = wc_get_order( $order_id );
		if ( ! $order || $order->get_payment_method() !== $this->id || $order->is_paid() ) {
			return;
		}
		$url = $order->get_meta( '_ebp_checkout_url' );
		if ( $url && $order->has_status( 'on-hold' ) ) {
			echo '<p><a class="button" href="' . esc_url( $url ) . '">Pay with Bitcoin</a></p>';
		}
	}

	private function reply( $code, $message ) {
		status_header( $code );
		header( 'Content-Type: application/json; charset=utf-8' );
		echo wp_json_encode( array( 'message' => $message ) );
		exit;
	}

	/** Called by the payment server: verify, re-check with the server, update the order. */
	public function handle_webhook() {
		$body   = file_get_contents( 'php://input' );
		$header = isset( $_SERVER['HTTP_X_EBP_SIGNATURE'] ) ? sanitize_text_field( wp_unslash( $_SERVER['HTTP_X_EBP_SIGNATURE'] ) ) : '';

		if ( ! EBP_Core::verify_signature( $this->get_option( 'webhook_secret' ), $header, $body, time() ) ) {
			$this->reply( 401, 'bad signature' );
		}
		$event = json_decode( $body, true );
		if ( ! is_array( $event ) || empty( $event['invoice_id'] ) ) {
			// Signed but not about an invoice (for example the dashboard's test message).
			$this->reply( 200, 'ok' );
		}
		$invoice_id = (string) $event['invoice_id'];

		$orders = wc_get_orders(
			array(
				'limit'      => 1,
				'meta_key'   => '_ebp_invoice_id', // phpcs:ignore WordPress.DB.SlowDBQuery
				'meta_value' => $invoice_id, // phpcs:ignore WordPress.DB.SlowDBQuery
			)
		);
		if ( empty( $orders ) ) {
			$this->reply( 200, 'unknown invoice' );
		}
		$order = $orders[0];

		// Source of truth: ask the server, do not trust the notification alone.
		$invoice = $this->client()->get_invoice( $invoice_id );
		if ( is_wp_error( $invoice ) ) {
			$this->reply( 503, 'could not confirm with the payment server' );
		}
		if ( EBP_Core::order_id_from_ref( isset( $invoice['order_id'] ) ? $invoice['order_id'] : '' ) !== $order->get_id() ) {
			$this->reply( 200, 'invoice does not belong to this order' );
		}

		$status = isset( $invoice['status'] ) ? (string) $invoice['status'] : '';
		$sats   = isset( $invoice['paid_sats'] ) ? (int) $invoice['paid_sats'] : 0;
		$total  = isset( $invoice['amount_sats'] ) ? (int) $invoice['amount_sats'] : 0;

		switch ( EBP_Core::action_for_status( $status ) ) {
			case 'paid':
				if ( ! $order->is_paid() && ! EBP_Core::amounts_match(
					isset( $invoice['fiat_currency'] ) ? $invoice['fiat_currency'] : '',
					isset( $invoice['fiat_amount'] ) ? $invoice['fiat_amount'] : '',
					$order->get_currency(),
					$order->get_total(),
					wc_get_price_decimals()
				) ) {
					if ( (string) $order->get_meta( '_ebp_mismatch_invoice' ) !== $invoice_id ) {
						$order->update_meta_data( '_ebp_mismatch_invoice', $invoice_id );
						$order->save();
						$order->add_order_note( 'A Bitcoin payment arrived, but the order total no longer matches the invoice it was made for. The order was NOT marked paid. Check the amount in your wallet and complete or refund it by hand.' );
					}
				} elseif ( ! $order->is_paid() ) {
					if ( $order->has_status( array( 'cancelled', 'failed', 'refunded' ) ) ) {
						$order->add_order_note( 'Bitcoin payment arrived after this order was cancelled. Check stock before shipping.' );
					}
					$order->add_order_note( sprintf( 'Bitcoin payment received (%d of %d sats).', $sats, $total ) );
					$order->payment_complete( $invoice_id );
				}
				break;
			case 'underpaid':
				if ( (string) $sats !== (string) $order->get_meta( '_ebp_underpaid_sats' ) ) {
					$order->update_meta_data( '_ebp_underpaid_sats', (string) $sats );
					$order->save();
					$order->add_order_note( sprintf( 'Partial Bitcoin payment received (%d of %d sats). Waiting for the rest.', $sats, $total ) );
				}
				break;
			case 'expired':
				if ( $order->has_status( 'on-hold' ) ) {
					$order->update_status( 'cancelled', 'The Bitcoin payment window ended without a full payment.' );
				}
				break;
		}
		$this->reply( 200, 'ok' );
	}
}
