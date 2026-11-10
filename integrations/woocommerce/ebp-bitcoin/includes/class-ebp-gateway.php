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
		$this->method_title       = __( 'Bitcoin (Easy Bitcoin Payments)', 'ebp-bitcoin' );
		$this->method_description = __( 'Accept Bitcoin through your own Easy Bitcoin Payments server. Customers pay on a hosted checkout page; the order is marked paid automatically.', 'ebp-bitcoin' );
		$this->has_fields         = false;
		$this->supports           = array( 'products' );

		$this->init_form_fields();
		$this->init_settings();

		$this->title       = $this->get_option( 'title', __( 'Bitcoin', 'ebp-bitcoin' ) );
		$this->description = $this->get_option( 'description', __( 'Pay with Bitcoin from any Bitcoin wallet app. You will finish on a secure payment page.', 'ebp-bitcoin' ) );

		add_action( 'woocommerce_update_options_payment_gateways_' . $this->id, array( $this, 'process_admin_options' ) );
		add_action( 'woocommerce_api_' . $this->id, array( $this, 'handle_webhook' ) );
	}

	public function webhook_url() {
		return add_query_arg( 'wc-api', $this->id, home_url( '/' ) );
	}

	public function init_form_fields() {
		$this->form_fields = array(
			'connect_steps'  => array(
				'title'       => __( 'Connect your shop in 3 steps', 'ebp-bitcoin' ),
				'type'        => 'title',
				'description' => '<ol style="list-style:decimal;margin-left:1.5em">'
					. '<li>' . esc_html__( 'ON YOUR PAYMENT SERVER (a different website from this one): open its admin page, your server address followed by /admin, and tap Connect in the bar at the bottom of the screen. Copy the address from card 1 into the Server address box below. For the API key, use the one you saved during setup; only if you lost it, make a new one in card 2.', 'ebp-bitcoin' ) . '</li>'
					. '<li>' . esc_html__( 'ON YOUR WOOCOMMERCE SITE (this page): copy "Your shop\'s notification address" shown further down this page. ON YOUR PAYMENT SERVER: paste it into card 3 ("Your shop\'s webhook address") and press Save address. A box labelled "Signing secret" appears once: press Copy, and save it in your password manager too. ON YOUR WOOCOMMERCE SITE: paste it into the Signing secret box below. No box? On your payment server press "New signing secret" instead.', 'ebp-bitcoin' ) . '</li>'
					. '<li>' . esc_html__( 'ON YOUR WOOCOMMERCE SITE: tick Enable and press Save changes. A green message means it works; a red one says what to fix. Then ON YOUR PAYMENT SERVER press Send a test notification on the Connect tab.', 'ebp-bitcoin' ) . '</li>'
					. '</ol>'
					. '<p><strong>' . esc_html__( 'Test version: use play-money (signet) coins only.', 'ebp-bitcoin' ) . '</strong></p>',
			),
			'enabled'        => array(
				'title'   => __( 'Enable', 'ebp-bitcoin' ),
				'type'    => 'checkbox',
				'label'   => __( 'Accept Bitcoin', 'ebp-bitcoin' ),
				'default' => 'no',
			),
			'server_url'     => array(
				'title'       => __( 'Server address', 'ebp-bitcoin' ),
				'type'        => 'text',
				'description' => __( 'Your payment server, from card 1 on the Connect tab of its admin page (a different website from this one). Example: https://pay.myshop.com', 'ebp-bitcoin' ),
				'desc_tip'    => false,
				'default'     => '',
				'placeholder' => 'https://',
			),
			'api_key'        => array(
				'title'       => __( 'API key', 'ebp-bitcoin' ),
				'type'        => 'password',
				'description' => __( 'The key you saved during setup. Lost it? On your payment server, make a new one: Connect tab, card 2.', 'ebp-bitcoin' ),
				'default'     => '',
			),
			'webhook_secret' => array(
				'title'       => __( 'Signing secret', 'ebp-bitcoin' ),
				'type'        => 'password',
				'description' => __( 'From your payment server: Connect tab, card 3 (shown once when you save the address).', 'ebp-bitcoin' ),
				'default'     => '',
			),
			'webhook_info'   => array(
				'title'       => __( 'Your shop\'s notification address', 'ebp-bitcoin' ),
				'type'        => 'title',
				'description' => __( 'Paste this into your payment server (Connect tab, card 3):', 'ebp-bitcoin' ) . '<br><code>' . esc_html( $this->webhook_url() ) . '</code>',
			),
			'title'          => array(
				'title'   => __( 'Name shown to customers', 'ebp-bitcoin' ),
				'type'    => 'text',
				'default' => __( 'Bitcoin', 'ebp-bitcoin' ),
			),
			'description'    => array(
				'title'   => __( 'Message shown to customers', 'ebp-bitcoin' ),
				'type'    => 'textarea',
				'default' => __( 'Pay with Bitcoin from any Bitcoin wallet app. You will finish on a secure payment page.', 'ebp-bitcoin' ),
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
			WC_Admin_Settings::add_error( __( 'Bitcoin: the server address is missing or does not start with https://', 'ebp-bitcoin' ) );
			return $saved;
		}
		if ( ! EBP_Core::is_secure_enough( $url ) ) {
			WC_Admin_Settings::add_error( __( 'Bitcoin: the server address should start with https:// so your API key is not sent in the clear.', 'ebp-bitcoin' ) );
		}
		$reach = EBP_Core::reachability_problem( $this->webhook_url() );
		if ( 'refused' === $reach ) {
			WC_Admin_Settings::add_error( __( 'Bitcoin: your shop\'s own address only works on this computer or network, so the payment server will refuse to send payment notices to it. Orders would never be marked paid. Use a shop with a public https:// address.', 'ebp-bitcoin' ) );
		} elseif ( 'probably_local' === $reach ) {
			WC_Admin_Settings::add_error( __( 'Bitcoin: your shop\'s address looks like one that only works on your own computer. If the payment server is on the internet it will not be able to send payment notices to it.', 'ebp-bitcoin' ) );
		}
		$client = new EBP_Client( $url, $this->get_option( 'api_key' ) );
		$health = $client->health();
		if ( is_wp_error( $health ) ) {
			WC_Admin_Settings::add_error( sprintf(
					/* translators: %s: the error message */
					__( 'Bitcoin: could not reach your payment server (%s).', 'ebp-bitcoin' ),
					$health->get_error_message()
				) );
			return $saved;
		}
		$key = $client->check_key();
		if ( is_wp_error( $key ) ) {
			WC_Admin_Settings::add_error( __( 'Bitcoin: the server answered, but the API key was not accepted. Make a new one in the dashboard and paste it here.', 'ebp-bitcoin' ) );
			return $saved;
		}
		if ( '' === $this->get_option( 'webhook_secret' ) ) {
			WC_Admin_Settings::add_error( __( 'Bitcoin: connected, but the signing secret is empty, so paid orders cannot be marked paid automatically.', 'ebp-bitcoin' ) );
			return $saved;
		}
		WC_Admin_Settings::add_message( __( 'Bitcoin: connected to your payment server.', 'ebp-bitcoin' ) );
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
			wc_add_notice( __( 'Bitcoin payment is not available right now. Please choose another method or try again shortly.', 'ebp-bitcoin' ), 'error' );
			return array( 'result' => 'failure' );
		}

		$order->update_meta_data( '_ebp_invoice_id', (string) $invoice['id'] );
		$order->update_meta_data( '_ebp_checkout_url', (string) $invoice['checkout_url'] );
		$order->update_status( 'on-hold', __( 'Waiting for the Bitcoin payment.', 'ebp-bitcoin' ) );
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
			echo '<p><a class="button" href="' . esc_url( $url ) . '">' . esc_html__( 'Pay with Bitcoin', 'ebp-bitcoin' ) . '</a></p>';
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
						$order->add_order_note( __( 'A Bitcoin payment arrived, but the order total no longer matches the invoice it was made for. The order was NOT marked paid. Check the amount in your wallet and complete or refund it by hand.', 'ebp-bitcoin' ) );
					}
				} elseif ( ! $order->is_paid() ) {
					if ( $order->has_status( array( 'cancelled', 'failed', 'refunded' ) ) ) {
						$order->add_order_note( __( 'Bitcoin payment arrived after this order was cancelled. Check stock before shipping.', 'ebp-bitcoin' ) );
					}
					$order->add_order_note( sprintf(
						/* translators: 1: sats received, 2: sats expected */
						__( 'Bitcoin payment received (%1$d of %2$d sats).', 'ebp-bitcoin' ),
						$sats,
						$total
					) );
					$order->payment_complete( $invoice_id );
				}
				break;
			case 'underpaid':
				if ( (string) $sats !== (string) $order->get_meta( '_ebp_underpaid_sats' ) ) {
					$order->update_meta_data( '_ebp_underpaid_sats', (string) $sats );
					$order->save();
					$order->add_order_note( sprintf(
						/* translators: 1: sats received so far, 2: sats expected */
						__( 'Partial Bitcoin payment received (%1$d of %2$d sats). Waiting for the rest.', 'ebp-bitcoin' ),
						$sats,
						$total
					) );
				}
				break;
			case 'expired':
				if ( $order->has_status( 'on-hold' ) ) {
					$order->update_status( 'cancelled', __( 'The Bitcoin payment window ended without a full payment.', 'ebp-bitcoin' ) );
				}
				break;
		}
		$this->reply( 200, 'ok' );
	}
}
