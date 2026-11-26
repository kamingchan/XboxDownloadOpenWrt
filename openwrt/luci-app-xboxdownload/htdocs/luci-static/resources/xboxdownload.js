'use strict';
'require baseclass';
'require rpc';
'require ui';
'require validation';

function method(name, params) {
	return rpc.declare({ object: 'luci.xboxdownload', method: name, params: params || [], reject: true,
		filter: function(result) {
			if (!result || result.ok === false) {
				var message = result && result.error || _('Request failed');
				var invalid = /^Invalid parameter: (\w+)$/.exec(message);
				throw new Error(invalid ? _('Invalid parameter: %s').format(invalid[1]) : _(message));
			}
			return result;
		}
	});
}

var groups = { xbox_cn1: 'Xbox CN1', xbox_cn2: 'Xbox CN2', xbox_app: 'Xbox App', akamai: 'Akamai' };
var families = { ipv4: 'IPv4', ipv6: 'IPv6', dual: _('Dual stack') };
var redirectDomains = [ 'assets1.xboxlive.com', 'assets2.xboxlive.com', 'xvcf1.xboxlive.com', 'xvcf2.xboxlive.com',
	'd1.xboxlive.com', 'd2.xboxlive.com', '5.xb.delivery.microsoft.com', 'assets1.xboxlive.com.edgesuite.net',
	'cl-glcb907925.gcdn.co', 'fg.microsoft.map.fastly.net', 'dlassets.xboxlive.com', 'dlassets2.xboxlive.com' ];
var srvDomains = [ 'assets-srvs.xboxlive.com', 'assets-srvs-net.trafficmanager.net', 'assets-srvs-apac.delivery.microsoft.com',
	'assets-srvs-na.delivery.microsoft.com' ];
var cdn = { xbox_cn1: [ 'assets1.xboxlive.cn', 'assets2.xboxlive.cn', 'd1.xboxlive.cn', 'd2.xboxlive.cn' ],
	xbox_cn2: [ 'dlassets.xboxlive.cn', 'dlassets2.xboxlive.cn' ],
	xbox_app: [ 'dl.delivery.mp.microsoft.com', 'tlu.dl.delivery.mp.microsoft.com', '1d.tlu.dl.delivery.mp.microsoft.com' ] };

function text(value) {
	return document.createTextNode(value == null || value === '' ? '—' : String(value));
}

function notify(message, level) {
	ui.addNotification(null, E('p', {}, text(message)), level || 'info');
}

function button(label, fn, style, disabled) {
	var b = E('button', { type: 'button', 'class': 'btn cbi-button ' + (style || ''), disabled: disabled || null,
		click: function() {
			b.disabled = true;
			Promise.resolve().then(fn).catch(function(err) { notify(err.message, 'error'); })
				.finally(function() { b.disabled = !!disabled; });
		} }, text(label));
	return b;
}

function table(headers, rows, empty) {
	return E('table', { 'class': 'xbox-table' }, [
		E('thead', {}, E('tr', {}, headers.map(function(h) { return E('th', { scope: 'col' }, text(h)); }))),
		E('tbody', {}, rows.length ? rows.map(function(row) {
			return E('tr', {}, row.map(function(v, i) {
				return E('td', { 'data-label': headers[i] }, v instanceof Node ? v : text(v));
			}));
		}) : [ E('tr', {}, E('td', { colspan: headers.length }, text(empty))) ])
	]);
}

function hasAddress(group, family) {
	if (!group || group.enabled !== '1' || (group.ip_family !== 'dual' && group.ip_family !== family)) return false;
	return family === 'ipv4' ? validation.parseIPv4(group.selected_ipv4 || '') : validation.parseIPv6(group.selected_ipv6 || '');
}

function rules(bind, port, groups) {
	var hosts = redirectDomains.concat(srvDomains);
	Object.keys(cdn).forEach(function(id) {
		var group = groups[id];
		if (!group || group.enabled !== '1') return;
		var selected = [ 'ipv4', 'ipv6' ].some(function(family) {
			if (group.ip_family !== 'dual' && group.ip_family !== family) return false;
			return hasAddress(group, family) || group.use_akamai_fallback === '1' && hasAddress(groups.akamai, family);
		});
		if (selected) hosts = hosts.concat(cdn[id]);
	});
	if (bind === '0.0.0.0') bind = '127.0.0.1';
	if (bind === '::') bind = '::1';
	var endpoint = (bind.indexOf(':') >= 0 ? '[' + bind + ']' : bind) + ':' + port;
	return {
		SmartDNS: [ 'server ' + endpoint + ' -group xboxdownload -exclude-default-group' ].concat(hosts.map(function(h) {
			return 'nameserver /' + h + '/xboxdownload';
		})).join('\n'),
		AdGuardHome: hosts.map(function(h) { return '[/' + h + '/]' + endpoint; }).join('\n'),
		dnsmasq: hosts.map(function(h) { return 'server=/' + h + '/' + bind + '#' + port; })
			.concat(redirectDomains.map(function(h) { return 'rebind-domain-ok=/' + h + '/'; })).join('\n')
	};
}

return baseclass.extend({
	status: method('status'), logs: method('logs', [ 'limit' ]),
	speedtest_start: method('speedtest_start', [ 'group', 'ip_family', 'filter_keyword', 'auto_apply' ]),
	speedtest_status: method('speedtest_status'), speedtest_cancel: method('speedtest_cancel'),
	apply_ip: method('apply_ip', [ 'group', 'ipv4', 'ipv6' ]), sync_ips: method('sync_ips', [ 'mirror', 'custom_base' ]),
	list_ips: method('list_ips', [ 'group', 'ip_family', 'filter_keyword' ]),
	save_custom_ips: method('save_custom_ips', [ 'group', 'text' ]), self_check: method('self_check'),
	suggest_upstream_dns: method('suggest_upstream_dns'),
	suggest_alias_ip: method('suggest_alias_ip', [ 'lan_device' ]), service_action: method('service_action', [ 'action' ]),
	groups: groups, families: families, rules: rules, text: text, button: button, notify: notify, table: table,
	stylesheet: function() { return E('link', { rel: 'stylesheet', href: L.resource('xboxdownload.css') }); },
	time: function(t) { return Number(t) > 0 ? new Date(t * 1000).toLocaleString() : '—'; },
	uptime: function(n) { return n == null ? '—' : _('%dh %dm').format(Math.floor(n / 3600), Math.floor(n / 60) % 60); },
	speed: function(n) { return n == null ? '—' : Number(n).toFixed(2) + ' MB/s'; },
	rtt: function(n) { return n == null ? '—' : Number(n).toFixed(1) + ' ms'; }
});
