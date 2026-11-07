<?php
/**
 * Runs when the plugin is deleted from WordPress. Removes everything the
 * plugin stored, including the API key and signing secret, so deleting the
 * plugin does not leave them behind in the database.
 *
 * Orders and their notes are not touched.
 *
 * @package EBP_Bitcoin
 */

if ( ! defined( 'WP_UNINSTALL_PLUGIN' ) ) {
	exit;
}

delete_option( 'woocommerce_ebp_bitcoin_settings' );
delete_option( 'ebp_bitcoin_issues' );
delete_option( 'ebp_bitcoin_health_failures' );
delete_transient( 'ebp_bitcoin_mailed_server_down' );
delete_transient( 'ebp_bitcoin_mailed_no_payments' );
wp_clear_scheduled_hook( 'ebp_bitcoin_watch' );
