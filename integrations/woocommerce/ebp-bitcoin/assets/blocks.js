( function () {
	var data = window.wc.wcSettings.getSetting( 'ebp_bitcoin_data', {} );
	var el = window.wp.element.createElement;
	var decode = window.wp.htmlEntities.decodeEntities;
	var label = decode( data.title || 'Bitcoin' );

	window.wc.wcBlocksRegistry.registerPaymentMethod( {
		name: 'ebp_bitcoin',
		label: el( 'span', null, label ),
		ariaLabel: label,
		content: el( 'div', null, decode( data.description || '' ) ),
		edit: el( 'div', null, label ),
		canMakePayment: function () {
			return true;
		},
		supports: { features: data.supports || [ 'products' ] },
	} );
} )();
